//! 唤醒问候语在会话层的落点：设备唤醒（`listen state=detect`）→ **立刻**说一句预制话术。
//!
//! 从 `session.rs` 拆出（见 `AGENTS.md` §5.10 文件规模约定）。`Session` 定义在父模块，
//! 子模块可直接访问其私有字段，因此拆分无需放宽任何可见性。
//!
//! ## 触发点为什么是 `listen detect`
//!
//! 固件在唤醒词命中后立刻发
//! `{"type":"listen","state":"detect","text":"<唤醒词>"}`（`Protocol::SendWakeWordDetected`，
//! 由 `CONFIG_SEND_WAKE_WORD_DATA=y` 启用）。这是"用户刚叫醒我"的**最早**信号：
//! 此刻用户还没说完第一句话，ASR 更没有结果。
//!
//! 加本模块之前，服务器收到这条消息只打一行日志（`listen state=detect`），于是用户唤醒后
//! 面对的是**完全静默**的等待——他要说完一句话，再等 VAD 切句、ASR、LLM 首 token，
//! 才第一次听到声音。把这段沉默换成一句「我在」，是体感上最廉价的一次提升。
//!
//! ## 为什么跳过 LLM
//!
//! 唤醒是全系统最高频的动作，而「我在」是**固定文本**——为一句固定的话付一次 LLM 调用
//!（延迟 + 计费 + 措辞长度不可控）没有道理。这里直接走预制话术通道
//!（[`Session::speak_canned`]，与指令闸门的告别语、LLM 失败的兜底话术同一条下行链路），
//! 一次 LLM 都不调。
//!
//! ## 音频复用：三级，越靠前越便宜
//!
//! ```text
//! ① 内存 TtsCache（跨会话，进程内）     命中 → 零合成，直接投递已编码帧
//! ② 磁盘 HotAudioStore（跨进程重启）    命中 → 读盘 ~2KB → 暖进 ① → 零合成
//! ③ 真合成（首次部署 / 换了音色）       合成一次 → 自动落盘 → 之后永久走 ①②
//! ```
//!
//! 稳态下每次唤醒都是**零 LLM、零 TTS**。第 ③ 级不是缺陷而是"换音色自动重新获取"的
//! 实现方式：缓存键含引擎签名（含音色凭据）、speaker、speed、下行采样率、帧长、文本，
//! 任一变化键即不同 → 不会命中旧音色的音频，而是重新合成并写一份新文件。
//! 不需要任何显式的失效逻辑——这正是把"格式/音色正确"做成构造性保证的收益。
//!
//! ## 三条不变量
//!
//! 1. **失败绝不影响对话**：问候语合成/下发失败只记 warn 就返回，主循环照常继续
//!   （用户接下来正常说话，只是没听到那句问候）。
//! 2. **每条 `detect` 都问候**（**没有**"每会话一次"的标志）：一次会话可以包含多次唤醒——
//!    指令闸门命中后用 MCP 让设备结束聆听时**音频通道刻意保留**（见
//!    [`crate::app::session::command`]），设备回到 idle 而会话还活着；用户再次唤醒时固件在
//!    同一条连接上再发一条 `detect`，那次也必须问候。用一次性标志会让"保活通道上的第二次
//!    唤醒静默无声"，正好废掉保活的意义。
//!    不需要防重复：固件的 `detect` 只从 `ContinueWakeWordInvoke` 发出，而它只在
//!    idle/notifying 状态下被调用（speaking/listening 时说唤醒词走 `SendStartListening`），
//!    因此每条 `detect` 都是一次真实唤醒；且本函数在主循环里被 `await`，天然不会重入。
//! 3. **关闭时零开销**：`[wakeup].enabled = false`（默认）时本函数在第一个判断处返回，
//!    不构造缓存键、不碰磁盘。
//!
//! ## 已知代价（实测前先说清）
//!
//! 问候语是**下行**音频，而它播放期间会话主循环正停在这个 `await` 里——上行帧（用户的第一
//! 句话）会在承载缓冲区里排队，等问候语下发完才被处理。因此问候语**越短越好**
//!（「我在」约 600ms，体感无感）；配置校验对超长问候语会拦下并说明这个代价。
//!
//! ⚠️ **与 MCP 能力发现的时序**：声明了 `features.mcp` 的设备，会话开始时先做一次能力发现
//!（`initialize` + `tools/list`，见 [`crate::app::session::mcp`]），期间上行帧**全部缓冲**
//! 后按原序补派发——本模块的触发帧（`detect`）就在这批缓冲里。因此问候语实际在**发现完成
//! 之后**才发出，代价 = 一次探测往返（局域网约 10–50ms；设备根本不回 MCP 报文时最多
//! `DISCOVERY_BUDGET` = 2s）。
//!
//! 为什么接受这个代价：发现必须在进入主循环**之前**做（主循环一开跑就没有插队发 MCP 请求的
//! 时机），而它在正常设备上只有两个往返，相对"设备建连（约 1s）+ 起跑闸（`lead_ms`）"可以
//! 忽略。若将来实测发现它确实拖慢了唤醒，正确的解法是**给能力发现加每设备 TTL**（复用注册表
//! 里已有的结果、跳过本次探测），而**不是**把问候语挪到发现之前——那会让"发 MCP 请求"与
//! "主循环已在读帧"撞在一起，正是发现逻辑刻意避开的场景。

use crate::app::hot_audio::{HotAudioKey, HotAudioStore};
use crate::app::transport::Transport;
use crate::app::tts_cache::TtsCache;

use super::Session;

/// 问候语音频的解析结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GreetingAudio {
    /// 成品帧已在内存缓存里就绪（内存或磁盘命中）；字符串是命中层级，**只用于日志**。
    Ready(&'static str),
    /// 需要真合成（首次部署 / 换了音色 / 关闭了落盘）。
    NeedsSynth,
}

/// 唤醒问候语的**音频来源决策**（纯函数，不依赖 `Session`）。
///
/// 刻意写成自由函数的原因与会话层其它逻辑一致：`Session` 需要真实 VAD 模型才能构造
///（`engine.rs` 的必需能力），把决策留在 `impl Session` 里就**只能靠真机验证**。
/// 抽出来之后，"内存命中 / 磁盘命中并暖进内存 / 都不命中 / 关闭落盘" 四条分支都能单测。
///
/// 副作用只有一个：**磁盘命中时把成品帧暖进内存缓存**——这样调用方随后的
/// `speak_canned` 会在既有预取路径上直接命中，零合成地走完整条下行链路
///（`tts start` → `sentence_start` → 帧 → `tts stop`），不需要为"缓存音频"再写一套下发。
pub(super) fn resolve_greeting_audio(
    cache: &TtsCache,
    store: &HotAudioStore,
    key: &HotAudioKey,
    persist: bool,
) -> GreetingAudio {
    let mem_key = key.mem_key();
    if cache.get(mem_key).is_some() {
        return GreetingAudio::Ready("内存缓存");
    }
    if persist {
        if let Some(frames) = store.get(key) {
            cache.insert(mem_key, frames);
            return GreetingAudio::Ready("磁盘缓存");
        }
    }
    GreetingAudio::NeedsSynth
}

impl<'a, T: Transport> Session<'a, T> {
    /// 唤醒问候：命中内存/磁盘缓存则零合成下发，否则合成一次并落盘。
    ///
    /// 返回 `()` 而不是 `Result` 是刻意的：问候语是**锦上添花**，它的任何失败都不该被
    /// 上抛成会话级错误（那会让一次本该正常的对话因为"问候没说出来"而中断）。
    pub(super) async fn handle_wake_greeting(&mut self) {
        // 不变量 3：未启用（默认）时零开销、零行为变化
        if !self.wake.active() {
            return;
        }
        // 测试台会话（macApp）不是设备：不问候。它的即时请求不该被一段问候语插队。
        if self.is_test {
            return;
        }

        let text = self.wake.text().to_string();
        let persist = self.wake.persist();
        // 键口径与合成路径**共用**同一构造入口（见 `HotAudioKey::from_session` 文档）：
        // 两处各写一遍必然漂移，而漂移的表现是"缓存永远不命中"——不报错，最难查。
        let key = HotAudioKey::from_session(
            &self.engines.live_config(),
            self.params.downlink_sr,
            self.params.downlink_frame_ms,
            &text,
        );
        let mem_key = key.mem_key();

        // ①② 决策（纯函数，见 `resolve_greeting_audio`）：内存命中 → 磁盘命中并暖进内存
        // → 都不命中则真合成。
        let audio = resolve_greeting_audio(
            &self.engines.tts_cache,
            &self.engines.hot_audio,
            &key,
            persist,
        );
        let reused = matches!(audio, GreetingAudio::Ready(_));

        if let GreetingAudio::Ready(source) = audio {
            tracing::info!(
                "session {} 唤醒问候命中{}（零 LLM / 零 TTS）: {text:?}",
                self.session_id,
                source
            );
        } else {
            tracing::info!(
                "session {} 唤醒问候首次合成（跳过 LLM，{}）: {text:?}",
                self.session_id,
                if persist {
                    "成功后落盘，之后免合成"
                } else {
                    "仅内存复用"
                }
            );
        }

        // ③ 走既有下行链路（命中即零合成：预取 worker 直接投递已编码帧）
        if let Err(e) = self.speak_canned(&text).await {
            tracing::warn!(
                "session {} 唤醒问候下发失败（不影响后续对话）: {e}",
                self.session_id
            );
            return;
        }

        // ④ 刚合成出来的 → 落盘（命中路径不需要，盘上已经有了）
        if !reused && persist {
            match self.engines.tts_cache.get(mem_key) {
                Some(audio) => {
                    if self.engines.hot_audio.put(&key, &audio.opus_frames) {
                        tracing::info!(
                            "session {} 唤醒问候已落盘（重启/下次唤醒免合成）: {text:?}",
                            self.session_id
                        );
                    } else {
                        tracing::warn!(
                            "session {} 唤醒问候未能落盘（下次唤醒仍走内存缓存）",
                            self.session_id
                        );
                    }
                }
                // 成品帧取不到 = `[tts].cache_entries == 0`（insert 直接丢弃）或本句被中断。
                // 前者已由配置校验拦下，这里再留一行 warn——**绝不静默失效**。
                None => tracing::warn!(
                    "session {} 唤醒问候已合成但取不到成品帧（[tts].cache_entries 是否为 0？），本次不落盘",
                    self.session_id
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests;
