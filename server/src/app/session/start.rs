//! 会话建立（`run_session`）：热刷新引擎 → 构造 VAD/解码器/全链路/闸门 → 登记注册表 → 进主循环。
//!
//! 从 `session.rs` 拆出（见 `AGENTS.md` §5.10 文件规模约定）。`Session` 定义在父模块，
//! **子模块可直接访问其私有字段**，因此拆分无需放宽任何可见性。

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::app::audio::opus::OpusFrameDecoder;
use crate::app::downlink::DownlinkPacer;
use crate::app::transport::Transport;
use crate::engine::Engines;
use crate::plugins::aiui::{AiuiSession, FullChainEngine};
use crate::plugins::command::CommandGate;
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
    params: SessionParams,
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
    let vad = engines.new_vad()?;
    let uplink_decoder = OpusFrameDecoder::new(params.uplink_sr)?;
    let real_audio = engines.config.real_audio();
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
    // 指令闸门快照：会话开始时取一次（= 注册表声明的 next_session 语义）。
    // 未启用时是恒不命中的空闸门，零开销、零行为变化。
    let commands = CommandGate::new(&engines.live_config().command);
    // 唤醒问候语快照：同上（next_session 语义）。
    let wake = WakeGreeting::new(&engines.live_config().wakeup);
    // 登记设备与会话（管理页「设备 / 会话 / 对话记录 / 概览」的数据源）。
    //
    // ⚠️ 位置在**所有可失败的初始化之后**：`new_vad()` / `OpusFrameDecoder::new()` 出错时
    // 会话根本没跑起来，不该在管理页留下一条"在线"的假连接。
    // 守卫类型是 RAII：`run_session` 返回（正常、`?` 提前返回、panic 展开）都会自动摘除活会话，
    // 因此这里**不需要**（也不该有）任何成对的清理代码。
    let _registry_guard = engines
        .registry
        .begin_session(crate::app::registry::SessionStart {
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
    let session = Session {
        transport,
        engines,
        params,
        session_id,
        device_id,
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
        commands,
        close_requested: false,
        mcp_enabled: mcp,
        mcp_next_id: 1,
        wake,
    };
    session.run().await
}
