//! 会话建立（`run_session`）：热刷新引擎 → 构造 VAD/解码器/全链路/闸门 → 登记注册表 → 进主循环。
//!
//! 从 `session.rs` 拆出（见 `AGENTS.md` §5.10 文件规模约定）。`Session` 定义在父模块，
//! **子模块可直接访问其私有字段**，因此拆分无需放宽任何可见性。

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::media::opus::OpusFrameDecoder;
use crate::workflow::downlink::DownlinkPacer;
use crate::observability::events::ServerEvent;
use crate::workflow::protocol::Transport;
use crate::runtime::Engines;
use crate::plugins::aiui::{AiuiSession, FullChainEngine};
use crate::plugins::skill::SkillRegistry;
use crate::plugins::wakeup::WakeGreeting;

use super::{Session, SessionParams};

/// 会话主循环：收发消息、驱动流水线。`is_test` 来自 hello 的 `test` 参数，
/// 只有测试台会话能发起 asr_test/tts_test/llm_test（设备正式流程不经过测试端点）。
///
/// `mcp` 来自 hello 的 `features.mcp`：决定本会话是否可以用 MCP 指挥设备。
/// 两个用途：① 会话开始时**能力发现**（`initialize` + `tools/list` → 管理界面，见 [`mcp`]）；
/// ② 指令闸门的「结束聆听」（[`command`]）。设备未声明 MCP 时两者都不发生（老固件向后兼容）。
///
/// `device_id`（`Device-Id` 头，固件写 MAC）与 `client_id`（`Client-Id` 头）都只用于
/// **观测与记忆作用域**，不参与协议协商：前者是设备稳定键（记忆隔离、管理页「设备」），
/// 后者是固件版本标识，仅进管理页「设备」列表。
pub async fn run_session<T: Transport>(
    transport: &mut T,
    engines: Arc<Engines>,
    mut params: SessionParams,
    session_id: String,
    device_id: String,
    client_id: String,
    is_test: bool,
    mcp: bool,
) -> anyhow::Result<()> {
    // 会话开始时按磁盘最新配置热刷新引擎（管理页保存后**新会话即生效**，无需重启）。
    // ⚠️ 必须在 spawn_blocking 内：sherpa 重建引擎可能加载模型数秒，直接调用会独占
    // tokio worker（见 engine.rs 模块注释的 CPU 预算）。刷新失败保持旧引擎，不致命。
    {
        let engines = engines.clone();
        let sid = session_id.clone();
        match tokio::task::spawn_blocking(move || engines.refresh_from_disk()).await {
            Ok(notes) => {
                for n in notes {
                    tracing::warn!("session {sid} 引擎热刷新：{n}");
                }
            }
            Err(e) => tracing::warn!("session {sid} 引擎热刷新任务异常: {e}"),
        }
    }
    let profile = transport.terminal_profile(&params);
    let bounded_turn = profile.capabilities.bounded_utterance;
    let mut turn_transport = super::utterance::TurnTransport::new(transport, bounded_turn);
    let transport = &mut turn_transport;
    let vad = engines.acquire_vad().await?;
    let uplink_decoder = if super::media::needs_opus(profile.input.codec) {
        Some(OpusFrameDecoder::new(params.uplink_sr)?)
    } else {
        None
    };
    let real_audio = engines.config.real_audio();
    // 每设备生效配置：全局 live_config ⊕ 本设备 overlay（白名单字段）。
    // 会话开始算一次，供命令/唤醒快照与请求时现读类共用（overlay 仅会话级生效）。
    let eff_config = engines.effective_config(&device_id);
    // Reset startup defaults before applying the latest session-pinned policy;
    // an older pre-handshake announcement must not stick after a live change.
    params.downlink_buffer_start_ms = engines
        .config
        .audio
        .downlink_buffer_start_ms
        .min(params.downlink_buffer_ms);
    params.downlink_buffer_start_max_ms = engines.config.audio.downlink_buffer_start_max_ms;
    eff_config.playback.apply(&mut params);
    let tts_lease = engines
        .tts_lease()
        .with_synthesis_params(eff_config.tts.speaker, eff_config.tts.speed);
    // 连接欢迎语（仅 EchoMuse 承载，见 `echomuse::connect_welcome`）：client_id 会被
    // 下面的 begin_session 按值消费，决策必须在其之前完成。
    let welcome = crate::device::echomuse::connect_welcome(&client_id, &eff_config.echomuse);
    // AIUI 全链路会话：sn = 前缀 + 会话短 id（云端个性化/上下文绑定，≤32 字符）
    // ⚠️ AIUI 全链路刻意用**启动快照**（注册表里是 RestartOnly）：它决定整条流水线的
    // 走向，且启用时凭据缺失是致命错误（boot 即失败）；热启用会带来"半配置切换链路"的风险。
    let aiui = if engines.config.aiui.enabled {
        let sn = format!(
            "{}-{}",
            engines.config.aiui.sn_prefix,
            session_id.get(..8).unwrap_or(&session_id)
        );
        let engine: Arc<Mutex<dyn FullChainEngine>> = Arc::new(Mutex::new(AiuiSession::new(
            engines.config.aiui.clone(),
            sn,
        )));
        Some(engine)
    } else {
        None
    };
    // 技能目录快照：会话开始时取一次（= 注册表声明的 next_session 语义）。
    // `[command]` + `[nlu]` 统一编译（规则 + 轻量模型两级，配置了 `[nlu].bert_*` 时
    // 第二级为 TinyBERT 语义分类、跨会话共享引擎）；全关时空目录，零开销。
    // 数据源用每设备生效配置。
    let bert = engines.bert_engine(&eff_config.nlu);
    let skills = SkillRegistry::new(&eff_config.command, &eff_config.nlu, bert);
    if skills.active() {
        tracing::info!(
            "session {session_id} 技能级联已启用：规则 + 轻量模型两级本地识别先于 LLM（设备 {device_id}）"
        );
    }
    // 唤醒问候语快照：同上（next_session 语义）。数据源用每设备生效配置。
    let wake = WakeGreeting::new(&eff_config.wakeup);
    // 登记设备与会话（管理页「设备 / 会话 / 对话记录 / 概览」的数据源）。
    //
    // ⚠️ 位置在**所有可失败的初始化之后**：`acquire_vad()` / `OpusFrameDecoder::new()` 出错时
    // 会话根本没跑起来，不该在管理页留下一条"在线"的假连接。
    // 守卫类型是 RAII：`run_session` 返回（正常、`?` 提前返回、panic 展开）都会自动摘除活会话，
    // 因此这里**不需要**（也不该有）任何成对的清理代码。
    // 事件总线（P0-5 叠加层）：会话建立即 emit（begin_session 之前，此时 client_id 尚未被
    // 下面的 begin_session 按值消费）——与 registry 的 note_* 并存，不影响任何流水线决策。
    engines.event_bus.emit(ServerEvent::SessionStart {
        session_id: session_id.clone(),
        device_id: device_id.clone(),
        client_id: client_id.clone(),
        is_test,
    });
    let _registry_guard = engines
        .registry
        .begin_session(crate::device::registry::SessionStart {
            session_id: session_id.clone(),
            device_id: device_id.clone(),
            client_id,
            uplink_sr: params.uplink_sr,
            downlink_sr: params.downlink_sr,
            downlink_frame_ms: params.downlink_frame_ms,
            bin_version: params.uplink_bin_ver.version_code() as u8,
            mcp,
            is_test,
        });
    // 注入通道（P1-2，见 `app/inject`）：仅**设备 + 级联流水线**会话注册——
    // 测试台走独立请求-响应（asr_test 等）、AIUI 全链路输入是 PCM（识别在云端闭环），
    // "文本进 LLM"对两者语义不通，不注册（HTTP 侧查无通道即报"该会话不支持注入"）。
    // guard 与 `_registry_guard` 同一 RAII 模式：`run_session` 返回即自动摘除。
    let (inject_tx, inject_rx) = tokio::sync::mpsc::channel(crate::workflow::pipeline::inject::CHANNEL_CAPACITY);
    let _inject_guard = if !is_test && aiui.is_none() {
        Some(engines.inject.register(&session_id, inject_tx))
    } else {
        None
    };
    let mut session = Session {
        transport,
        engines,
        params,
        session_id,
        device_id,
        eff_config,
        tts_lease,
        bounded_turn,
        pending_segments: Vec::new(),
        history: Vec::new(),
        downlink_ts: 0,
        test_recording: false,
        test_buf: Vec::new(),
        vad,
        uplink_decoder,
        real_audio,
        is_test,
        abort: Arc::new(AtomicBool::new(false)),
        aiui,
        pacer: DownlinkPacer::new(params.downlink_lead_ms as u64),
        skills,
        close_requested: false,
        mcp_enabled: mcp,
        mcp_next_id: 1,
        wake,
        inject_rx: if _inject_guard.is_some() {
            Some(inject_rx)
        } else {
            None
        },
    };
    // 连接欢迎语（`[echomuse].welcome`，RestartOnly）：设备接入即说一次，走唤醒问候
    // 同一预制话术链路（`speak_canned`）——固定文本合成一次即经预取链路落盘
    // hot-audio/（裸 PCM 文件，引擎原样、无重采样，见 `media::hot_audio`），稳态零合成；失败只记日志，
    // 绝不影响会话本身。
    if let Some(text) = welcome {
        if let Err(e) = session.speak_canned(&text).await {
            tracing::warn!(
                "session {} 连接欢迎语下发失败（不影响会话）: {e}",
                session.session_id
            );
        }
    }
    session.run().await
}
