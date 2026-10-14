//! 轻量事件总线（P0-5）：会话在关键节点 `emit` 进程级事件。
//!
//! ## 定位
//!
//! 这是一条**叠加观测层**：与 `observe.rs` 的 `note_*` 注册表埋点**并存**，不是替换——
//! `note_*` 写的是"当前在线/会话状态"这种有状态的观测面（面板直接读），而事件总线是
//! 一条**无状态、可多订阅者**的扇出通道（用于大盘 / 审计 / 未来扩展）。两者字段口径尽量
//! 对齐（session_id / device_id），但互不影响、不共享代码。
//!
//! ## 为什么用 `tokio::sync::broadcast`
//!
//! - 一对多扇出：一个会话 emit，N 个订阅者（大盘 / 日志 / 指标）各自收全量；
//! - 解耦：会话**完全不感知**谁在听，新增订阅者不必改会话代码；
//! - `send` 失败（无订阅者 / 慢消费者 LAGGED）被忽略——观测面绝不允许把语音链路带崩。
//!
//! ## 容量
//!
//! 通道容量固定 1024。这是"慢消费者"容忍上限：订阅者处理不过来会丢最旧事件（LAGGED），
//! 但发布者永不阻塞。P0 仅有调试订阅者，1024 足够；未来大盘若需零丢失，再评估换方案。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::broadcast;

/// 轮次标识：进程内自增、从 1 开始的整数。
///
/// ## 为什么是 `u64` 而不是 UUID
///
/// - **长度**：一轮至少 6 条日志（轮次开始/ASR/首 token/首句/TTS 完成/轮次结束），
///   UUID 的 36 字符在这些行里是纯格式化开销与纯噪声（本机是 N5105 + `cpus: "3.5"`）；
/// - **可排序**：自增天然单调，`turn=3` 必早于 `turn=4`，把一轮日志 `sort` 出来时间线自洽；
/// - **够用**：唯一性范围只需「进程内」，与现有观测面口径完全一致——`/api/metrics`
///   明写「内存态，进程重启清零」，`registry` / `usage` 同理。把 id 做成跨进程唯一
///   没有消费者；将来真要跨进程聚合，用 `(session_id, turn_id)` 组合即可
///   （`session_id` 本身已是 uuid，见 `session/start.rs`）。
///
/// 取值从 **1** 起：`0` 留作「无轮次」哨兵，日志里出现 `turn=0` 一眼可辨为异常。
pub type TurnId = u64;

/// 进程级轮次自增计数器（`Relaxed` 即可：唯一性靠原子 RMW，不参与其他同步关系）。
static NEXT_TURN_ID: AtomicU64 = AtomicU64::new(1);

/// 分配下一个轮次标识（[`TurnTrace::new`] 的唯一入口，进程内唯一且单调递增）。
fn next_turn_id() -> TurnId {
    NEXT_TURN_ID.fetch_add(1, Ordering::Relaxed)
}

/// 轮次延迟锚点：`speech_end`（VAD 切句）| `inject`（管理台注入文本到达）|
/// `canned`（兜底/问候/告别语入口）| `aiui`（全链路段落起点）。
///
/// 唯一权威在此（[`TurnStages::anchor`] 的取值集合）；各锚点轮次各自成事件、
/// 靠该字段区分（不混窗——"兜底话术没有 LLM 阶段"不该拉低正常轮次的 LLM 分位）。
pub const ANCHOR_SPEECH_END: &str = "speech_end";
/// 管理台注入（`POST /api/devices/{id}/inject`）文本到达时刻为锚点。
pub const ANCHOR_INJECT: &str = "inject";
/// 非 LLM 预制话术（LLM 失败兜底 / 指令告别语 / 唤醒问候语）入口时刻为锚点。
pub const ANCHOR_CANNED: &str = "canned";
/// AIUI 全链路（云端闭环）的段落起点为锚点。
pub const ANCHOR_AIUI: &str = "aiui";

/// 一轮对话的分段延迟快照（事件负载；全部为**相对锚点的毫秒偏移**，
/// `Instant` 单调钟，mark 时刻才算 elapsed）。
///
/// 各阶段 `None` = 本轮未发生（如注入轮没有 ASR；缓存命中轮没有合成首 PCM）。
#[derive(Clone, Debug)]
pub struct TurnStages {
    /// 锚点语义：见 [`ANCHOR_SPEECH_END`] 等四个常量（锚点本身即 t0，偏移恒 0，
    /// 以该字段显式标注、不单列数值）。
    pub anchor: &'static str,
    /// ASR 识别完成（语音轮专属；锚点 = speech_end）。
    pub asr_end_ms: Option<u64>,
    /// LLM 首个文本 token 到达。
    pub llm_first_token_ms: Option<u64>,
    /// 首个完整句子送 TTS（含定时 flush 出的残句）。
    pub sentence_ready_ms: Option<u64>,
    /// 首个**非缓存**句的首个 PCM 分片回调（缓存命中轮为 `None`——零合成无从谈首 PCM）。
    pub tts_first_pcm_ms: Option<u64>,
    /// 首帧音频实际下发（既有 TTFA 日志口径，`sender.rs` 的 `first_frame_at`）。
    pub first_audio_ms: Option<u64>,
}

impl TurnStages {
    fn fresh(anchor: &'static str) -> Self {
        Self {
            anchor,
            asr_end_ms: None,
            llm_first_token_ms: None,
            sentence_ready_ms: None,
            tts_first_pcm_ms: None,
            first_audio_ms: None,
        }
    }

    /// 分段的一行可读摘要（轮次结束日志用）：各阶段毫秒偏移，未发生的阶段显示 `-`。
    ///
    /// 存在的意义：`grep turn=3` 捞出一轮的所有日志行后，**最后一行就能直接回答
    /// 「慢在哪一段」**，不必把 5 条 mark 日志的字段手工拼起来看。
    pub fn summary(&self) -> String {
        let ms = |v: Option<u64>| match v {
            Some(x) => format!("{x}ms"),
            None => "-".to_string(),
        };
        format!(
            "asr={} llm首字={} 首句={} 首pcm={} 首帧={}",
            ms(self.asr_end_ms),
            ms(self.llm_first_token_ms),
            ms(self.sentence_ready_ms),
            ms(self.tts_first_pcm_ms),
            ms(self.first_audio_ms),
        )
    }
}

/// 分段延迟记录器：`Arc` 共享，producer / 合成回调 / 发送端各自 mark。
///
/// ## 一个实例 = 一个轮次
///
/// 轮次标识在 [`TurnTrace::new`] 内部分配（见 [`next_turn_id`]），因此「一个 trace 就是一个
/// 轮次」是**构造性保证**：轮次 id 随 `Arc` 一起流转，调用方不可能传错或漏传。
/// 4 个构造点（speech_end / inject / canned / aiui）覆盖全部轮次入口。
///
/// 各字段 mark 均 **first-only**（`None → Some` 才写，重复 mark 无效）、瞬时持锁
///（锁内无 await，`std::sync::Mutex` 足够）。时间口径见 [`TurnStages`]。
/// 锚点语义随 [`TurnStages::anchor`] 快照带出（构造时写入，不在本结构单独存一份）。
pub struct TurnTrace {
    /// 本轮标识（构造时分配，此后不变）：事件负载与日志的公共归因键。
    turn_id: TurnId,
    t0: Instant,
    inner: Mutex<TurnStages>,
}

impl TurnTrace {
    /// 以锚点语义创建（t0 = 调用时刻，调用方须在锚点事件发生的同 tick 内调用——
    /// 语音轮在 `recognize_segments` 每段循环入口，与 VAD 回调误差微秒级）。
    ///
    /// 顺带分配一个进程内唯一的 [`TurnId`]（本轮此后所有日志与事件都用它）。
    pub fn new(anchor: &'static str) -> Arc<Self> {
        Arc::new(Self {
            turn_id: next_turn_id(),
            t0: Instant::now(),
            inner: Mutex::new(TurnStages::fresh(anchor)),
        })
    }

    /// 本轮标识（`Copy`，可随意复制进闭包/日志宏）。
    pub fn turn_id(&self) -> TurnId {
        self.turn_id
    }

    /// first-only mark；返回 `true` = **本次调用真正写入了**（即这是该阶段的首次 mark）。
    ///
    /// 返回值让「打首 token / 首句就绪日志」的判据与 mark **同源**：调用点不必另设一个
    /// 原子标志（那种写法会与 first-only 语义各自漂移，一旦语义改动就出现「日志说到了、
    /// 阶段却是 None」这类自相矛盾的观测）。
    fn mark(&self, field: fn(&mut TurnStages) -> &mut Option<u64>) -> bool {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let slot = field(&mut g);
        if slot.is_some() {
            return false;
        }
        *slot = Some(self.t0.elapsed().as_millis() as u64);
        true
    }

    /// ASR 识别完成（语音轮）。返回是否首次写入（见 [`TurnTrace::mark`]）。
    pub fn mark_asr_end(&self) -> bool {
        self.mark(|s| &mut s.asr_end_ms)
    }

    /// LLM 首个文本 token 到达。返回是否首次写入（见 [`TurnTrace::mark`]）。
    pub fn mark_llm_first_token(&self) -> bool {
        self.mark(|s| &mut s.llm_first_token_ms)
    }

    /// 首个完整句子送 TTS。返回是否首次写入（见 [`TurnTrace::mark`]）。
    pub fn mark_sentence_ready(&self) -> bool {
        self.mark(|s| &mut s.sentence_ready_ms)
    }

    /// 首个非缓存句的首个 PCM 分片回调。返回是否首次写入（见 [`TurnTrace::mark`]）。
    pub fn mark_tts_first_pcm(&self) -> bool {
        self.mark(|s| &mut s.tts_first_pcm_ms)
    }

    /// 首帧音频实际下发：按**传入的首帧时刻**计 elapsed（不是 mark 调用时刻）。
    ///
    /// 为什么必须带时刻参数：调用方（`drain_prepared`/`aiui_downlink`）在
    /// `drain_reservoir` **返回后**才走到这里，此刻整轮末帧都已发完——用"现在"
    /// 记出来的 first_audio 会系统性失真成"接近整轮耗时"。真实首帧时刻由
    /// 发送端（`downlink/sender.rs`）在首帧写线时填进 `first_frame_at` out-param，
    /// 这里消费它。`None`（未发出任何帧，如起跑闸即被打断）不 mark。
    ///
    /// 返回是否首次写入（见 [`TurnTrace::mark`]）。
    pub fn mark_first_audio_at(&self, at: Option<Instant>) -> bool {
        let Some(at) = at else { return false };
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.first_audio_ms.is_some() {
            return false;
        }
        g.first_audio_ms = Some(at.duration_since(self.t0).as_millis() as u64);
        true
    }

    /// 当前快照（emit 时取；读锁内纯拷贝）。
    pub fn snapshot(&self) -> TurnStages {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests;

/// 进程级事件：会话生命周期与一轮对话的关键节点。
///
/// 所有字段均为只读快照，**不含任何密钥 / 业务决策状态**——订阅者拿到的只是"发生了什么"
/// 的事实，不能据此改变流水线行为（决策仍在会话内部）。
///
/// 部分字段（会话/设备标识、轮次细节）当前订阅者（`app/metrics`）不逐字段读取——
/// 它们是事件**契约**的一部分（审计/大盘订阅者按需消费），删掉即破坏 wire 契约，
/// 故对负载统一豁免 dead_code 而不是删除字段。
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum ServerEvent {
    /// 会话建立（握手完成、引擎热刷新后、进入主循环前）。
    SessionStart {
        session_id: String,
        device_id: String,
        client_id: String,
        is_test: bool,
    },
    /// 会话结束（主循环退出、标记离线前）。`reason` 为轻量字符串：
    /// `"closed"` = 承载正常关闭；`"command"` = 指令闸门命中断开本次会话。
    SessionEnd {
        session_id: String,
        device_id: String,
        reason: String,
    },
    /// 一条消息入对话（用户说话 / 助手整轮下发完成）。`elapsed_ms` 仅助手轮次有
    /// （整轮耗时 LLM + 合成 + 下行），用户轮次为 `None`。
    ///
    /// `turn_id`：**用户轮次与其对应的助手轮次是同一个 `turn_id`**——它们同属一轮对话。
    /// 类型是 `Option` 而非裸值：事件负载是**契约**（审计/大盘订阅者按需消费），
    /// 契约不该被会话侧"目前总会传值"这种实现事实绑死；生产路径经
    /// `session::observe::emit_message_added` 收敛，只会传 `Some`。
    MessageAdded {
        session_id: String,
        device_id: String,
        /// 本条消息所属轮次（用户与其助手回复共享同一个值）。
        turn_id: Option<TurnId>,
        role: String,
        text: String,
        elapsed_ms: Option<u64>,
    },
    /// 一轮 TTS 下行完成（含兜底话术 / 唤醒问候语这类非 LLM 句）：句数 / 帧数 / 是否打断 / 音频时长。
    TtsRound {
        session_id: String,
        device_id: String,
        /// 本轮标识（与同轮的 `TurnLatency` / `MessageAdded` 一致，可直接串联）。
        turn_id: TurnId,
        sentences: u32,
        frames: u32,
        interrupted: bool,
        audio_ms: u64,
    },
    /// 会话级异常（LLM 调用失败 / 任务异常等）。
    Error {
        session_id: String,
        device_id: String,
        message: String,
    },
    /// 一轮对话的分段延迟快照（缺口②）：各阶段相对锚点（speech_end/inject/canned/aiui）
    /// 的毫秒偏移，订阅者（`app/metrics`）聚合分位。`interrupted` = 本轮被打断。
    /// 兜底/问候/告别语各自成事件（anchor=canned），不与正常回复混窗。
    ///
    /// ## 计数契约（订阅者必须遵守）
    ///
    /// **一轮 = 恰好一条 `TurnLatency`**，因此每个阶段窗口每轮只进 **1 个样本**。
    /// 这条事件里的 5 个阶段是同一轮的 5 次采样，**不是 5 轮**——订阅者若按"分段条数"
    /// 计数，一轮就会灌 5 个样本进每个窗口，分位数被同一轮自身重复加权而失真。
    /// 会话侧的保证来自 `stream_response` 的三个**互斥**结束分支（各自 emit 后即返回）；
    /// 聚合侧的保证由 `app/metrics` 的测试钉死。
    TurnLatency {
        session_id: String,
        device_id: String,
        /// 本轮标识：把这一组分段归因到具体某轮（配合 `tracing` 的 `turn=` 字段定位现场）。
        turn_id: TurnId,
        stages: TurnStages,
        interrupted: bool,
    },
}

/// 事件总线：基于 `tokio::sync::broadcast` 的进程级扇出通道。
///
/// 持有 `Sender`，`subscribe()` 产出 `Receiver`。`emit` 忽略一切发送失败——
/// 无订阅者（`SendError`）或慢消费者（`Err(Lagged)`）都不应影响发布者（会话）的运行。
pub struct EventBus {
    tx: broadcast::Sender<ServerEvent>,
}

impl EventBus {
    /// 构造总线。`capacity` 为通道缓冲上限（见模块注释，P0 用 1024）。
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// 新增一个订阅者（各自独立收全量事件）。
    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.tx.subscribe()
    }

    /// 发布一条事件；**忽略**发送结果（无订阅者 / LAGGED 都不影响发布者）。
    pub fn emit(&self, e: ServerEvent) {
        let _ = self.tx.send(e);
    }
}
