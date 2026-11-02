//! 每连接会话状态机与语音流水线。
//!
//! 生产：音频 → ASR → 一次真实模型意图 → 校验后的技能动作，或无工具纯聊天 → TTS。
//! 不做规则/启发式识别，也不允许 LLM 重选技能；缺槽位只明确澄清。
//! 设备定时消息由独立 outbox 投递，不伪装成用户话语。
//! 测试台 hello.test=true 仅受理独立 ASR/TTS/LLM/NLU 请求，不投递任务消息。
//!
//! ## 与设备的关系（MCP 方向，勿反）
//!
//! **设备是 MCP Server、服务器是 MCP Client**：设备的工具（音量/屏幕/灯…）由设备注册，
//! 服务器在**既有那条 WS** 上发 `tools/call`。协议原语见 [`crate::workflow::mcp`]。
//!
//! 会话开始时还会做一次**能力发现**（`initialize` + `tools/list`，见 [`mcp`] 子模块）：
//! 结果写进 [`crate::workflow::mcp::registry`]，管理界面「MCP 工具」页据此显示"这台设备支持
//! 哪些工具、每个能干什么"。发现期间的上行帧全部缓冲后补派发，一帧不丢。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::config::Config;
use crate::media::opus::OpusFrameDecoder;
use crate::observability::events::{TurnId, TurnTrace};
use crate::plugins::aiui::FullChainEngine;
use crate::plugins::asr::AsrEngine;
use crate::plugins::skill::SkillRegistry;
use crate::plugins::wakeup::WakeGreeting;
use crate::runtime::Engines;
use crate::workflow::downlink::DownlinkPacer;
use crate::workflow::protocol::{BinVersion, ClientMessage, ServerMessage};
use crate::workflow::protocol::{IncomingFrame, Transport};

/// 一条用户话语在进入 LLM 前的决策结果（文本侧管线 P0-4 的 Stop 信号，见
/// `Session::decide_user_text`）。
enum UtteranceDecision {
    /// 继续 Stage 3（LLM 流水线 / AIUI 下行）。
    Respond,
    /// NLU 命中已知意图：副作用（告别语 + MCP 退下 / 本地指令应答）**已在 Stage 2 内执行完**，
    /// 外层段循环应终止——本批剩余语音段一并丢弃（用户已要求退下 / 已本地消费）。
    NluStop,
}

mod aiui;
mod alarm;
mod bench;
mod command;
mod delivery;
mod delivery_audio;
mod hooks;
mod mcp;
mod media;
mod observe;
mod plan;
mod producer;
mod reply;
mod skill_plan;
mod start;
mod utterance;
mod wakeup;

// 上行 PCM 采样格式换算（AIUI 路径用）：下移到 app/audio/pcm（行为不变搬移）。
pub(crate) use crate::media::pcm::f32_to_i16_16k;

// 会话入口对 `gateway::ws` 可见（网关把升级后的连接交给会话状态机）。
pub use start::run_session;

/// 在 `spawn_blocking` 中执行同步阻塞的 CPU 密集调用（ASR/TTS 推理），
/// 统一处理任务取消/panic 与内部错误，返回 `Err(String)`（含可读错误信息）。
/// 调用方据此决定降级行为（warn 后 continue 或 return）。
async fn blocking_result<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(format!("推理失败: {e:#}")),
        Err(e) => Err(format!("推理任务异常（可能 panic）: {e}")),
    }
}

/// 接入入口声明的设备类型：**入口即定型**。
///
/// 两个生产入口各自声明，会话层据此走专用分支（设备端 VAD 直通、连接欢迎语注入等），
/// 不再从音频格式/codec 反推；共享流水线环节（ASR/技能/LLM/TTS/下行节拍）不感知该类型：
/// - [`crate::gateway::ws`]（xiaozhi hello 协商入口）→ [`DeviceKind::XiaozhiEsp`]；
/// - [`crate::device::echomuse`]（`/em/control`+`/em/data`，identify/ack、无 hello）→ [`DeviceKind::EchoMuse`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    /// xiaozhi ESP 承载（含 `hello.test=true` 的 macApp 测试台——同一入口、同一 hello 协商）。
    XiaozhiEsp,
    /// EchoMuse 承载（Echo Dot 2 / emOS 直连；原生 mono PCM，设备端已做 speech gate）。
    EchoMuse,
}

/// 会话协商参数（握手后固定）。
#[derive(Clone, Copy)]
pub struct SessionParams {
    /// 接入入口声明的设备类型（见 [`DeviceKind`]）：会话专用分支的唯一判据。
    pub kind: DeviceKind,
    pub uplink_bin_ver: BinVersion,
    pub downlink_bin_ver: BinVersion,
    pub uplink_sr: u32,
    pub downlink_sr: u32,
    pub output_codec: crate::media::media::AudioCodec,
    pub downlink_frame_ms: u32,
    /// 下行抖动缓冲提前量（毫秒），见 `[audio].downlink_lead_ms`。
    pub downlink_lead_ms: u32,
    /// 服务器侧下行水库容量（毫秒），见 `[audio].downlink_buffer_ms`。
    pub downlink_buffer_ms: u32,
    /// 下载缓冲预算（毫秒，由水库容量派生），见 `[audio].download_budget_ms`。
    pub download_buffer_ms: u32,
    /// 起跑闸（毫秒）：首帧前先攒够这么多音频，见 `[audio].downlink_buffer_start_ms`。
    pub downlink_buffer_start_ms: u32,
    /// 起跑闸最长等待（毫秒），见 `[audio].downlink_buffer_start_max_ms`。
    pub downlink_buffer_start_max_ms: u32,
}

/// LLM 失败且用户尚未听到任何内容时的兜底回应（**不暴露技术细节**，只给可行动的安抚）。
const FALLBACK_REPLY: &str = "抱歉，我这边网络不太顺畅，请稍后再试。";

/// 单连接会话：承载 [`Transport`]、引擎引用与全部可变会话状态。
///
/// 流水线函数（`handle_text`/`handle_binary`/`stream_response`/`send_tts_audio`/
/// `recognize_segments`）原本要传 7~11 个离散参数（含多个可变状态），增删一个状态
/// 就要改 N 处签名。聚合成 `Session` 后统一用 `&mut self` 访问，消除参数膨胀与漏传风险。
/// 泛型 `T: Transport` 静态分发：会话/流水线不感知承载协议（当前 WS，未来
/// MQTT+UDP / 本机通道各实现一份 [`Transport`] 即可复用整条流水线）。
pub struct Session<'a, T: Transport> {
    transport: &'a mut T,
    engines: Arc<Engines>,
    params: SessionParams,
    session_id: String,
    /// 设备标识（握手头 `Device-Id`）：记忆作用域的稳定键（`xiaozhi:<device_id>`）
    /// 与 `{{device_id}}` 插值的取值来源。
    device_id: String,
    /// 每设备生效配置（全局 `live_config()` ⊕ 设备 overlay 白名单字段）。
    /// 会话开始时算一次；请求时现读类与命令/唤醒快照类都走它，使 per-device overlay 生效。
    eff_config: Arc<Config>,
    /// 固定实际 TTS 实例与身份；NextSession 不在句间读取进程最新引擎。
    tts_lease: crate::runtime::TtsLease,
    memory: Arc<dyn crate::plugins::memory::MemoryProvider>,
    memory_completed_through: Option<TurnId>,
    /// 明确结束标记的承载延迟 ASR 到边界，NoSpeech 才能无副作用丢弃。
    bounded_turn: bool,
    pending_segments: Vec<Vec<f32>>,
    history: Vec<(String, String)>,
    downlink_ts: u32,
    test_recording: bool,
    test_buf: Vec<f32>,
    /// 服务端 VAD 实例（池借出，RAII 归还）。`None` = 设备端已做 VAD 的会话（EchoMuse
    /// speech gate，见 `start.rs::device_vad`）：跳过服务端检测，`device_vad=true` 走直通缓冲。
    vad: Option<crate::plugins::vad::pool::VadGuard>,
    /// 设备端已做 VAD（EchoMuse speech gate 判定人声才上行）：上行 PCM 直接缓冲，
    /// `UtteranceEnd::Speech`（设备 0x04）时整段送 ASR——不做服务端二次检测。
    device_vad: bool,
    uplink_decoder: Option<OpusFrameDecoder>,
    real_audio: bool,
    /// hello.test=true 的测试台会话：唯一能发起 asr_test/tts_test/llm_test 的连接。
    is_test: bool,
    /// 打断标志（barge-in）：`handle_text` 收 abort、或流水线轮询到 socket 中 abort/断开时置位。
    /// 生产者（LLM 读流）与消费者（TTS 合成/下行）各阶段轮询，一轮对话结束重置。
    abort: Arc<AtomicBool>,
    /// 全链路引擎（[aiui].enabled 时启用；跨轮次长连接，Mutex 供 spawn_blocking 互斥）。
    aiui: Option<Arc<Mutex<dyn FullChainEngine>>>,
    /// 下行节拍器：按帧长匀速下发（保护 ESP 解码队列，见 `downlink` 模块文档）。
    /// 一次回复内跨句共享时间表（句间无缝）；每轮回复开始时 reset。
    pacer: DownlinkPacer,
    /// 会话固定目录 + 共享模型。只有真实模型的一次决策拥有技能授权。
    skills: SkillRegistry,
    skill_turn_id: TurnId,
    delivery_idle: bool,
    workflow: crate::workflow::designer::CompiledWorkflow,
    /// 指令要求断开本次会话（告别语已下发）：主循环收尾时主动关闭承载。
    close_requested: bool,
    /// 设备 hello 是否声明 `features.mcp`（最新固件硬编码 `true`；极老固件为 `false`）。
    ///
    /// `false` 时本会话**一个 MCP 报文都不发**（零开销、零行为变化），指令闸门直接走
    /// 「断开本次会话」的降级路径——这是对老固件的向后兼容承诺。
    mcp_enabled: bool,
    /// 下一个 MCP JSON-RPC `id`（自增；响应靠它配对，见 [`mcp`] 子模块）。
    mcp_next_id: u64,
    /// 唤醒问候语（`[wakeup]`）：设备唤醒（`listen state=detect`）时立刻说的预制话术。
    /// 会话开始时按配置快照构造一次 = 注册表声明的 `next_session` 语义。
    ///
    /// ⚠️ **没有"每会话只问候一次"的标志**：一次会话可以包含**多次唤醒**——指令闸门命中后
    /// 我们用 MCP 让设备结束聆听，**音频通道刻意保留**（见 [`command`] 子模块），
    /// 设备回到 idle 而会话还活着；用户再次唤醒时固件在同一条连接上再发一条 `detect`，
    /// 那次也必须问候。用一次性标志会让"保活通道上的第二次唤醒静默无声"，
    /// 正好废掉保活的意义。
    ///
    /// 不需要防重复：固件的 `detect` 只从 `ContinueWakeWordInvoke` 发出，而它只在
    /// **idle/notifying** 状态下被调用（`HandleWakeWordDetectedEvent` 在 speaking/listening
    /// 时走 `SendStartListening`）——因此每条 `detect` 都是一次真实唤醒；且本函数在会话
    /// 主循环里被 `await`，天然不会重入。
    wake: WakeGreeting,
    /// 管理台注入通道（P1-2，见 `app/inject`）：`Some` = 本会话可被注入（设备 + 级联流水线），
    /// `None` = 测试台 / AIUI 全链路（不注册，HTTP 侧报"不支持"）。主循环 `select!` 消费。
    inject_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    /// 到期定时任务的动作直推通道（见 `app/device_cmds`）：`Some` = 本会话可接收
    /// scheduler 的设备工具调用命令（与 inject 同一注册门槛）。主循环 `select!` 消费。
    cmd_rx: Option<tokio::sync::mpsc::Receiver<crate::workflow::device_cmds::DeviceCmd>>,
}

/// [`Session::inject_rx`] 为 `None` 时的 select 分支：永不完成的 future（让该分支纯摆设），
/// 避免在主循环里为"可注入与否"写两套循环。
async fn inject_next(rx: &mut Option<tokio::sync::mpsc::Receiver<String>>) -> Option<String> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

/// [`Session::cmd_rx`] 为 `None` 时的 select 分支（同 [`inject_next`] 的摆设模式）。
async fn cmd_next(
    rx: &mut Option<tokio::sync::mpsc::Receiver<crate::workflow::device_cmds::DeviceCmd>>,
) -> Option<crate::workflow::device_cmds::DeviceCmd> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

impl<'a, T: Transport> Session<'a, T> {
    /// 主循环：接收上行帧并分发；循环结束后 flush VAD 残留段并送流水线。
    async fn run(mut self) -> anyhow::Result<()> {
        // MCP 能力发现（仅 hello 声明 `features.mcp` 的**设备**会话）：必须在进入主循环**之前**做
        // ——设备建连后立刻开始上行音频，主循环一旦开跑就没有"插队发 MCP 请求"的时机。
        // 期间到达的上行帧由发现逻辑**全部缓冲**，下面按原顺序补派发（一帧不丢）。
        // 测试台会话（`hello.test=true`，macApp）不探测：它不是设备，不该进设备工具注册表，
        // 也不该为 `asr_test` 这类即时请求付探测延迟。
        let prelude = if self.mcp_enabled && !self.is_test {
            self.discover_mcp_tools().await
        } else {
            Vec::new()
        };

        let mut closed = false;
        for frame in prelude {
            if !self.handle_frame(frame).await {
                closed = true;
                break;
            }
        }
        let mut delivery_tick = tokio::time::interval(std::time::Duration::from_secs(1));
        delivery_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while !closed && !self.close_requested {
            // 双路等待：上行帧（正常语音/协议消息）与管理台注入文本（P1-2，见 `app/inject`）。
            // `transport.recv()` 基于 `socket.next().await`（取消安全）：select 竞争失败
            // 只是放弃等待，帧不丢；注入到达时若流水线正忙，会等当前帧处理完才被消费
            // （mpsc 排队，串行化避免与语音轮次并发写下行）。
            tokio::select! {
                biased;
                frame = self.transport.recv() => {
                    if !self.handle_frame(frame).await {
                        break;
                    }
                }
                _ = delivery_tick.tick() => {
                    if self.delivery_idle && !self.is_test && self.aiui.is_none() {
                        self.deliver_pending_message().await;
                    }
                }
                injected = inject_next(&mut self.inject_rx) => {
                    if let Some(text) = injected {
                        self.handle_inject(&text).await;
                    }
                }
                cmd = cmd_next(&mut self.cmd_rx) => {
                    if let Some(cmd) = cmd {
                        self.handle_device_cmd(cmd).await;
                    }
                }
            }
        }
        // NLU 命中 end_session **且 MCP 未能让设备结束聆听**：降级为断开本次会话。
        // （正常路径下设备已自己退出聆听，`close_requested` 为 false，会话继续留在通道上。）
        if self.close_requested {
            tracing::info!("session {} 按指令降级断开本次会话", self.session_id);
        }

        // 会话结束：flush VAD 残留语音段（真实音频模式），尽力把最后一句话送完流水线。
        // 连接可能已关闭，此时发送会失败并被记录，属正常情况。
        // ⚠️ 指令断开会话时**不** flush：用户已明确要求退下，不该再为残留音频开一轮对话。
        if self.real_audio && !self.close_requested && !self.bounded_turn {
            let mut segments = Vec::new();
            if let Some(vad) = self.vad.as_mut() {
                vad.flush(&mut |seg| segments.push(seg));
            }
            self.recognize_segments(segments).await;
        }
        if self.close_requested {
            // 告别语的下行帧是**按实时节奏**下发的：最后一帧发出时，设备解码队列里还有约
            // `downlink_lead_ms` 的音频没播完（见 `downlink` 模块的节拍器文档）。立即关闭连接
            // 有可能把告别语的尾巴掐掉，因此留一段等长的排空时间（夹在 100ms~1s，避免异常
            // 配置把会话收尾拖住）。失败无妨——会话即将随 Transport 释放而结束。
            let drain_ms = self.params.downlink_lead_ms.clamp(100, 1000) as u64;
            tokio::time::sleep(std::time::Duration::from_millis(drain_ms)).await;
            self.transport.close().await;
        }
        // 会话结束：把该设备标为离线（管理界面「MCP 工具」据此区分"在线 / 上次探测结果"）。
        // ⚠️ 放在最末：VAD flush 与告别语排空期间设备仍在线上。
        // 事件总线（P0-5 叠加层）：会话结束。刻意**不**塞进 registry 的 SessionGuard::Drop
        // ——观测面（registry）不反向依赖事件层，两者解耦（见 `app/events` 模块注释）。
        self.engines
            .event_bus
            .emit(crate::observability::events::ServerEvent::SessionEnd {
                session_id: self.session_id.clone(),
                device_id: self.device_id.clone(),
                reason: if self.close_requested {
                    "command".to_string()
                } else {
                    "closed".to_string()
                },
            });
        self.engines
            .mcp
            .mark_offline(&self.device_id, &self.session_id);
        Ok(())
    }

    /// 派发一条上行帧；返回 `false` 表示承载已断开（会话应结束）。
    ///
    /// 主循环与 MCP 探测后的**补派发**共用这一处：两条路径对"文本 / 二进制 / 断开"的处置
    /// 必须完全一致，否则被缓冲过的帧会走出与正常流程不同的分支（例如音频绕过 VAD）。
    async fn handle_frame(&mut self, frame: IncomingFrame) -> bool {
        match frame {
            IncomingFrame::Text(s) => {
                if let Err(e) = self.handle_text(&s).await {
                    tracing::warn!("处理文本消息失败: {e}");
                }
            }
            IncomingFrame::Binary(b) => self.handle_binary(&b).await,
            IncomingFrame::Audio(audio) => self.handle_audio(audio).await,
            IncomingFrame::AudioBoundary(reason, epoch) => {
                let reason = if epoch.is_current() {
                    reason
                } else {
                    crate::workflow::protocol::UtteranceEnd::NoSpeech
                };
                self.handle_utterance_end(reason).await;
            }
            IncomingFrame::EndOfUtterance(reason) => {
                self.handle_utterance_end(reason).await;
            }
            // 承载关闭/读取错误：会话结束（recv 内已记日志）
            IncomingFrame::Closed => return false,
        }
        true
    }

    /// scheduler 直推的到期任务动作（见 `app/device_cmds`）：在会话循环内执行设备
    /// 工具调用——transport 独占权在这里，MCP 等回复才能与上行帧复用同一条连接。
    /// 语义与计划内 `devices.call` 完全一致（含 `has_tool` 已声明能力校验，复用
    /// [`Self::dispatch_skill_device_tool`]）；结果经 oneshot 回给 scheduler，
    /// 由它决定 outbox 里播成功原文还是失败提示，本函数不产生任何语音副作用。
    async fn handle_device_cmd(&mut self, cmd: crate::workflow::device_cmds::DeviceCmd) {
        match cmd {
            crate::workflow::device_cmds::DeviceCmd::CallDeviceTool {
                tool,
                arguments,
                reply,
            } => {
                let result = self.dispatch_skill_device_tool(&tool, arguments).await;
                if let Err(e) = &result {
                    tracing::warn!(
                        "session {} 到期任务动作直推失败（tool={tool}）: {e}",
                        self.session_id
                    );
                }
                let _ = reply.send(result);
            }
        }
    }

    /// 管理台注入的一轮（P1-2）：文本**作为用户话语**走与语音完全相同的管线入口
    /// （`decide_user_text` 记录+指令闸门 → `respond` 统一响应入口：LLM 或回声→TTS→下行）。
    ///
    /// 这让"上游 LLM/TTS 是否正常"可以不经 ASR 直接验证（联调利器），且对话记录、
    /// 事件总线、打断语义（设备仍可 abort）与正常轮次一一一致。
    ///
    /// 分段延迟（缺口②）：anchor=`inject`（文本到达时刻），无 asr/sentence 之前的
    /// 语音阶段——`asr_end_ms` 为 `None`，LLM 及之后各阶段照常采集。
    async fn handle_inject(&mut self, text: &str) {
        let trace = TurnTrace::new(crate::observability::events::ANCHOR_INJECT);
        tracing::info!(
            turn = trace.turn_id(),
            "session {} 管理台注入文本（{} 字），走完整管线",
            self.session_id,
            text.chars().count()
        );
        match self.decide_user_text(text, trace.turn_id()).await {
            // 副作用（告别语 + MCP 结束聆听 / close_requested 置位）已在闸门内执行完，
            // 主循环下一轮即按 close_requested 收尾——语义与语音触发的指令一致。
            UtteranceDecision::NluStop => {}
            UtteranceDecision::Respond => {
                if let Err(e) = self.respond(text, &trace).await {
                    tracing::warn!(
                        turn = trace.turn_id(),
                        "session {} 注入轮次流水线失败: {e}",
                        self.session_id
                    );
                }
            }
        }
    }

    /// 在 `spawn_blocking` 中执行同步阻塞的 ASR 识别，统一处理任务取消/panic 与内部错误。
    /// `recognize_segments` 与 `AsrTest` 共用，避免重复 clone 引擎与计时样板。
    /// 写成不借用 `&self` 的关联函数：避免捕获 `Session`（含 `&mut T` 传输，非 `Sync`）
    /// 导致返回的 future 不满足 `Send`，破坏 `ws.rs` 的 `on_upgrade` 要求。
    ///
    /// 推理闸门（缺口①）：先过 ASR 闸门（裸 acquire，**不查 abort**——`abort` 只在
    /// `stream_response` 入口复位，而 ASR 发生在它之前，stale abort 不能吞掉
    /// "打断后立刻说的新话语"，见 `app/inference` 模块注释）。permit move 进
    /// `spawn_blocking` 闭包、随推理结束 RAII 归还。
    async fn asr_recognize(
        asr: Arc<dyn AsrEngine>,
        asr_gate: crate::workflow::pipeline::inference::GateHandle,
        sr: u32,
        pcm: Vec<f32>,
    ) -> Result<String, String> {
        let Some(_permit) = asr_gate.acquire().await else {
            return Err("ASR 推理闸门已关闭".to_string());
        };
        blocking_result(move || {
            // permit 随本闭包 Drop 归还闸门
            asr.recognize(&pcm, sr)
        })
        .await
    }

    /// 将 VAD 切出的语音段送 ASR → 完整流水线；常规分帧与会话结束 flush 共用。
    async fn recognize_segments(&mut self, segments: Vec<Vec<f32>>) {
        let session_id = self.session_id.clone();
        // AIUI 全链路模式：切段即轮次，识别+大模型+合成在 AIUI 云端闭环
        if self.aiui.is_some() {
            self.aiui_segments(segments).await;
            return;
        }
        for seg in segments {
            // 分段延迟（缺口②）：锚点 = speech_end（VAD 切句、即将送 ASR 的时刻，
            // 与 VAD 回调同 tick，误差微秒级）。trace 构造即分配本轮 `turn_id`
            // （见 `app/events`），此后本轮所有日志与事件都带它。
            let trace = TurnTrace::new(crate::observability::events::ANCHOR_SPEECH_END);
            let turn = trace.turn_id();
            tracing::debug!(
                turn,
                "session {session_id} 轮次开始（锚点 speech_end，{} 样本）",
                seg.len()
            );
            let t0 = std::time::Instant::now();
            let user_text = match Self::asr_recognize(
                self.engines.asr.clone(),
                self.engines.inference.asr(),
                self.params.uplink_sr,
                seg,
            )
            .await
            {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(turn, "session {session_id} ASR 识别失败: {e}");
                    continue;
                }
            };
            if user_text.trim().is_empty() {
                continue;
            }
            trace.mark_asr_end();
            // 对话内容入日志（用户侧）：默认 info 档的"用户说了什么"唯一出口。
            tracing::info!(
                turn,
                "session {session_id} 用户：{user_text}（识别 {}ms）",
                t0.elapsed().as_millis()
            );
            // 文本侧优先级管线（P0-4）：记录 → 指令闸门 → LLM（统一入口，见 decide_user_text）
            match self.decide_user_text(&user_text, turn).await {
                UtteranceDecision::NluStop => return, // 本批剩余语音段一并丢弃
                UtteranceDecision::Respond => {
                    if let Err(e) = self.respond(&user_text, &trace).await {
                        tracing::warn!(turn, "session {session_id} 流水处理失败: {e}");
                    }
                }
            }
        }
    }

    /// 用户话语：先记录，再一次模型授权，最后（仅未消费时）无工具聊天。
    /// AIUI 仍在云端结果后进入，唤醒 detect 与定时 outbox 不经过此入口。
    async fn decide_user_text(&mut self, user_text: &str, turn_id: TurnId) -> UtteranceDecision {
        // Stage 1：记录（注册表观测 + 事件总线叠加层，见 observe.rs / events.rs）
        self.abort.store(false, Ordering::Relaxed); // new user turn, not a receipt
        self.skill_turn_id = turn_id;
        self.note_user_turn(user_text);
        self.emit_message_added(turn_id, "user", user_text, None);
        // Stage 2: the only intent decision; no LLM action nomination follows.
        let consumed = self.handle_skill_gate(user_text).await;
        self.delivery_idle = true;
        if consumed {
            UtteranceDecision::NluStop
        } else {
            UtteranceDecision::Respond
        }
    }

    /// 测试台专用端点（`hello.test=true` 才受理）：非测试会话收到一律忽略并记日志。
    /// 返回 `true` 表示当前会话是测试台、可继续处理；`false` 表示已记日志忽略、调用方应跳过。
    fn accept_test_only(&self, endpoint: &str) -> bool {
        if self.is_test {
            true
        } else {
            tracing::warn!(
                "session {} 非测试会话收到 {}，忽略",
                self.session_id,
                endpoint
            );
            false
        }
    }

    async fn handle_text(&mut self, text: &str) -> anyhow::Result<()> {
        let session_id = self.session_id.clone();
        let msg: ClientMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("解析客户端消息失败: {e}");
                return Ok(());
            }
        };
        match msg {
            ClientMessage::Listen { state, .. } if state == "start" => {
                // ESP 正式流程：listening 状态由设备侧 VAD 上报触发，语音段经二进制帧上行，
                // 服务端 VAD 切句后走 ASR → LLM → TTS 流水线（见 handle_binary）。
                // debug：唤醒/按键事件已在 detect 行打印（来源随 text 透传），本行只是
                // 状态机细节，默认 info 档不重复刷屏（排障 `RUST_LOG=debug` 可见）。
                tracing::debug!("session {session_id} 进入 listening");
                // 观测：管理页「会话」里能看到"这台设备正在聆听"（只记录，不影响流水线）
                self.delivery_idle = false;
                self.note_phase(crate::device::registry::Phase::Listening);
            }
            ClientMessage::Listen { state, text, .. } if state == "detect" => {
                // **设备唤醒**（固件 `Protocol::SendWakeWordDetected`，由
                // `CONFIG_SEND_WAKE_WORD_DATA=y` 启用）：这是"用户刚叫醒我"最早、最可靠的
                // 信号——比任何 ASR 结果都早。在加 `[wakeup]` 之前这条消息只打一行日志，
                // 用户唤醒后面对的是完全静默的等待。处置见 [`wakeup`] 子模块。
                // `text` 在 ESP 上是唤醒词；echomuse 合成 detect 时用它携带触发来源
                // （"本地唤醒词"/"action 键"，见 device::echomuse::turn），故措辞用
                // "唤醒词/来源"兼容两种承载。
                tracing::info!(
                    "session {session_id} 收到唤醒上报（唤醒词/来源：{:?}）",
                    text.as_deref().filter(|t| !t.is_empty()).unwrap_or("未知")
                );
                self.delivery_idle = false;
                self.run_wake_hooks(text.as_deref()).await?;
            }
            ClientMessage::Listen { state, .. } => {
                if state == "stop" {
                    self.delivery_idle = true;
                }
                tracing::info!("listen state={state}");
            }
            ClientMessage::Abort { .. } => {
                tracing::info!("session {session_id} 收到 abort（置位打断标志）");
                self.abort.store(true, Ordering::Relaxed);
            }
            ClientMessage::Mcp { payload, .. } => {
                // 设备侧 MCP 响应/通知。**不再回显**（旧实现把设备发来的 payload 原样发回去，
                // 设备因缺 `method` 直接丢弃——纯占位逻辑，见 docs/esp-compat-and-latency-plan.md §3.3）。
                // 走到这里说明这条消息**没有等待方**：迟到的响应（超时后才回）或设备主动通知。
                // 只记日志、不回包——回显会让设备日志刷 "Missing method"，掩盖真正的问题。
                tracing::info!(
                    "session {session_id} 收到设备 MCP 消息（无等待方，仅记录）: {}",
                    crate::workflow::mcp::summarize(&payload)
                );
            }
            // 测试台专用端点（仅 hello.test=true 受理）；实现见 session/bench.rs。
            // 非测试会话收到一律忽略（accept_test_only 记日志后返回 false）。
            ClientMessage::AsrTest { action, .. } => {
                if self.accept_test_only("asr_test") {
                    self.handle_asr_test(action).await?;
                }
            }
            ClientMessage::TtsTest {
                text,
                speaker,
                lang,
                speed,
                vcn,
                ..
            } => {
                if self.accept_test_only("tts_test") {
                    self.handle_tts_test(text, speaker, lang, speed, vcn)
                        .await?;
                }
            }
            ClientMessage::LlmTest { text, .. } => {
                if self.accept_test_only("llm_test") {
                    self.handle_llm_test(text).await?;
                }
            }
            ClientMessage::NluTest { text, .. } => {
                if self.accept_test_only("nlu_test") {
                    self.handle_nlu_test(text).await?;
                }
            }
            ClientMessage::Hello(_) => {
                tracing::warn!("会话内收到重复 hello，忽略");
            }
        }
        Ok(())
    }
}

/// 协议层发送：`ServerMessage` 序列化后交承载下发（错误映射在承载实现内完成）。
pub(crate) async fn send_text<T: Transport>(
    transport: &mut T,
    msg: &ServerMessage,
) -> anyhow::Result<()> {
    transport.send_text(msg.to_json()).await
}
