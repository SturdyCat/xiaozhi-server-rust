//! 采集指令的唯一出口：`mic_start`/`mic_stop`/`beam_*` 全部经此下发。
//!
//! 官方时序（`controller/em_controller.py` 的 `_button_voice_turn` + 固件
//! `data.go` 注释「the controller sends that pair after every voice turn」）：
//!
//! - 接入后：`mic_start`（无 `lock_mic`）跑 ch6 全向**常开唤醒流**——波束基线、
//!   AEC、影子评分的持续输入。缺了它，turn 流刚起就 `beam.Lock()` 必然落在
//!   `baseline not ready` 分支，锁到一个测不到能量的方向（真机 20dB 落差的来源）。
//! - 开轮：`mic_stop → beam_lock → mic_start{lock_mic:true}`。先停常开流是硬要求：
//!   固件 `StartMic` 对已激活的流直接忽略（`already active — ignoring`），
//!   `0x04` 路径的 `streamMic` 不自行退出，不停就永远起不了 turn 流。
//! - 收口：`mic_stop → beam_unlock → mic_start`（回常开流）。
//!
//! 指令全部走 control 通道（FIFO 保序）；通道未绑定（测试/未接入）或已关闭时
//! 静默失败并返回 `false`，调用方据此收口聆听门。

use std::sync::OnceLock;

use tokio::sync::mpsc::UnboundedSender;

use super::ControlOut;

#[derive(Debug, Default, Clone)]
pub(crate) struct MicControl {
    ctl: OnceLock<UnboundedSender<ControlOut>>,
}

impl MicControl {
    /// 绑定 control 出站通道（control_session 在创建 channel 后调用一次）。
    pub(crate) fn bind(&self, ctl: UnboundedSender<ControlOut>) {
        let _ = self.ctl.set(ctl);
    }

    fn send(&self, json: &'static str) -> bool {
        self.ctl
            .get()
            .is_some_and(|c| c.send(ControlOut::Json(json.into())).is_ok())
    }

    /// 常开唤醒流：omni（ch6）、无门控、AGC 关，服务端评分/波束基线的持续输入。
    pub(crate) fn wake_stream(&self) -> bool {
        self.send(r#"{"type":"mic_start"}"#)
    }

    /// 停当前 mic 流（幂等：固件对未激活流是 no-op）。
    pub(crate) fn stop(&self) -> bool {
        self.send(r#"{"type":"mic_stop"}"#)
    }

    /// 锁定说话人所在的外圈麦（官方在唤醒命中/开轮时发）。
    pub(crate) fn beam_lock(&self) -> bool {
        self.send(r#"{"type":"beam_lock"}"#)
    }

    /// 回全向（一轮结束后）。
    pub(crate) fn beam_unlock(&self) -> bool {
        self.send(r#"{"type":"beam_unlock"}"#)
    }

    /// bounded turn 流：VAD 门控 + preroll，流开时固件会 `beam.Lock()`。
    pub(crate) fn turn_stream(&self) -> bool {
        self.send(r#"{"type":"mic_start","lock_mic":true}"#)
    }

    /// 一轮收口（官方 `mic_stop → beam_unlock → mic_start` 三连）。
    /// 不依赖 TTS 是否已播完：常开流在 turn 不活跃期间的上行会被服务端丢弃，
    /// 立即恢复换来的是"绝不会漏恢复"——漏恢复等价于设备永久卡死。
    pub(crate) fn turn_ended(&self) {
        self.stop();
        self.beam_unlock();
        self.wake_stream();
    }
}
