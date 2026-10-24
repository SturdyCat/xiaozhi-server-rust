//! 灯环与提示音/打断（control 面 JSON；`led_anim` 由设备本地渲染，抖动无关）。
//!
//! 形状依据：设备端 dispatch `case "led_anim"`（`anim` 原样转发给本地动画引擎，
//! schema 由服务器侧拥有：`{pattern, colors, periodMs?, ttlSec, listening?}`）、
//! `case "play_cue"`（`cue`，今天只有 `"wake"`）、`case "speaker_flush"`。
//! 配色取官方 `em_scenes.py` 的 `standard` 预设：聆听绿 (0,180,0)、spinner
//! head/trail (0,200,0)/(0,60,0)；静音红环是设备本地覆盖，永不被这里画掉。
//!
//! `ttlSec` 是死信开关：控制器/链路死掉后环自动清，不给用户留一个永远转圈。

use std::sync::OnceLock;

use tokio::sync::mpsc::UnboundedSender;

use super::ControlOut;

/// spinner 头/尾色（官方 standard 预设）。
const SPIN_HEAD: [u8; 3] = [0, 200, 0];
const SPIN_TRAIL: [u8; 3] = [0, 60, 0];
/// 聆听实心绿。
const LISTENING: [u8; 3] = [0, 180, 0];
/// 死信上限：长回复绝不许环在中途自己清掉（官方 METER_TTL_PAD 注释的原则）。
const SPEAKING_TTL: u64 = 120;

#[derive(Debug, Default, Clone)]
pub(crate) struct Fx {
    ctl: OnceLock<UnboundedSender<ControlOut>>,
}

impl Fx {
    /// 绑定 control 出站通道（与 [`super::mic::MicControl`] 同一条 FIFO，保序）。
    pub(crate) fn bind(&self, ctl: UnboundedSender<ControlOut>) {
        let _ = self.ctl.set(ctl);
    }

    /// 以既有通道构造（transport 侧：已持有 ctl_tx 克隆）。
    pub(crate) fn bound(ctl: UnboundedSender<ControlOut>) -> Self {
        let fx = Self::default();
        fx.bind(ctl);
        fx
    }

    fn send(&self, json: String) -> bool {
        self.ctl
            .get()
            .is_some_and(|c| c.send(ControlOut::Json(json)).is_ok())
    }

    /// 聆听态：实心绿环。`listening:true` 标记聆听环（方向 overlay 依赖它）。
    pub(crate) fn listening(&self) -> bool {
        self.send(
            serde_json::json!({
                "type": "led_anim",
                "anim": {"pattern": "solid", "colors": [LISTENING], "listening": true, "ttlSec": 30},
            })
            .to_string(),
        )
    }

    /// 思考态：本地 spinner（80ms/拍），10s 死信。
    pub(crate) fn thinking(&self) -> bool {
        self.send(
            serde_json::json!({
                "type": "led_anim",
                "anim": {"pattern": "spin", "colors": [SPIN_HEAD, SPIN_TRAIL], "periodMs": 80, "ttlSec": 10},
            })
            .to_string(),
        )
    }

    /// 播报态：环随播放电平起伏（设备侧 ALSA 写入处 RMS）。
    pub(crate) fn speaking(&self) -> bool {
        self.send(
            serde_json::json!({
                "type": "led_anim",
                "anim": {"pattern": "meter", "colors": [SPIN_HEAD], "ttlSec": SPEAKING_TTL},
            })
            .to_string(),
        )
    }

    /// 无语音收口：一次慢脉冲——"我听着，但什么都没听到"（1s 自清）。
    pub(crate) fn nospeech(&self) -> bool {
        self.send(
            serde_json::json!({
                "type": "led_anim",
                "anim": {"pattern": "pulse", "colors": [SPIN_HEAD], "periodMs": 900, "ttlSec": 1},
            })
            .to_string(),
        )
    }

    /// 熄灯：12 颗全 0 的一帧 `leds`（meter/死信之外的手动清场，播报收尾用）。
    pub(crate) fn off(&self) -> bool {
        let leds: Vec<serde_json::Value> = (0..12)
            .map(|id| serde_json::json!({"id": id, "r": 0, "g": 0, "b": 0}))
            .collect();
        self.send(serde_json::json!({"type": "leds", "leds": leds, "listening": false}).to_string())
    }

    /// 唤醒提示音（`wakeSound` 开启时设备才发声；0-TTFA，先于 TTS）。
    pub(crate) fn wake_cue(&self) -> bool {
        self.send(serde_json::json!({"type": "play_cue", "cue": "wake"}).to_string())
    }

    /// 掐断设备侧缓冲的播报（打断：新开一轮必须立刻静掉上一轮的回答，
    /// 否则设备里已缓冲的音频会播完——"打断了还在说"）。
    pub(crate) fn speaker_flush(&self) -> bool {
        self.send(r#"{"type":"speaker_flush"}"#.into())
    }

    // ── 音乐面（voice turn 与背景音乐的混音协调）──
    // 注意：music_flush 只用于"用户真的停止"——它会丢弃设备里缓冲的音乐，
    // 不可恢复；语音轮次绝不发它（那正是 duck 存在的原因）。

    /// 语音轮次开始：把音乐压低 `duckDb`（config 键，设备侧执行）。
    pub(crate) fn duck_on(&self) -> bool {
        self.send(r#"{"type":"duck","on":true}"#.into())
    }

    /// 语音轮次结束：恢复音乐。
    pub(crate) fn duck_off(&self) -> bool {
        self.send(r#"{"type":"duck","on":false}"#.into())
    }

    /// 用户停止播放：丢弃设备缓冲的音乐（不可恢复，语音轮次禁用）。
    pub(crate) fn music_flush(&self) -> bool {
        self.send(r#"{"type":"music_flush"}"#.into())
    }
}
