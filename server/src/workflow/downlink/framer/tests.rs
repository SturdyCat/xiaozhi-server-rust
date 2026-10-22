//! `StreamingFramer`（`server/src/app/downlink/framer.rs`）的单元测试。
//!
//! 未启用 `sherpa` 时 Opus 编码器是占位实现（返回空帧），因此断言只看**帧数与切分
//! 位置**，不看负载字节——切分逻辑才是这里要钉住的东西。

use super::StreamingFramer;

/// 24k → 24k（最常见路径）：2880 样本 = 2 个 1440 样本的整帧。
#[test]
fn pusher_splits_whole_frames_at_same_rate() {
    let mut f = StreamingFramer::new(24000, 24000, 60).expect("建立分帧器");
    assert_eq!(f.frame_samples(), 1440, "24k/60ms = 1440 样本");
    let frames = f.push(&vec![0.1f32; 2880]).expect("分帧");
    assert_eq!(frames.len(), 2);
    assert!(f.finish().expect("收尾").is_none(), "整帧切完不应有尾帧");
}

/// 分片边界不得影响切分结果：逐片推送与一次性推送等价（相位/`carry` 跨片连续）。
#[test]
fn chunk_boundaries_do_not_change_frame_split() {
    let chunk = vec![0.25f32; 1000];
    let mut one = StreamingFramer::new(24000, 24000, 60).expect("建立分帧器");
    let mut many = StreamingFramer::new(24000, 24000, 60).expect("建立分帧器");

    let whole = one.push(&vec![0.25f32; 3000]).expect("整段分帧");
    let mut split: Vec<Vec<u8>> = Vec::new();
    for _ in 0..3 {
        split.extend(many.push(&chunk).expect("逐片分帧"));
    }
    assert_eq!(whole.len(), split.len(), "切分帧数应一致");
    assert_eq!(whole.len(), 2, "3000 样本 → 2 整帧 + 120 样本尾");
    assert_eq!(one.frame_samples(), many.frame_samples());
}

/// 升采样（TTS 16k → 下行 24k）：帧长按**下行**采样率算，残样本补零成尾帧。
#[test]
fn upsample_splits_by_downlink_frame() {
    let mut f = StreamingFramer::new(16000, 24000, 60).expect("建立分帧器");
    // 1600 样本 @16k = 100ms → 2400 样本 @24k = 1 整帧（1440）+ 960 残样本
    let frames = f.push(&vec![0.0f32; 1600]).expect("分帧");
    assert_eq!(frames.len(), 1, "2400 样本 = 1 整帧 + 残");
    let tail = f.finish().expect("收尾").expect("应有尾帧");
    assert!(
        !tail.is_empty() || cfg!(not(feature = "sherpa")),
        "尾帧应已编码（无 sherpa 时是占位空帧）"
    );
}

/// 空分片 / 空收尾都不产帧（避免下发零长度二进制帧）。
#[test]
fn empty_input_yields_no_frames() {
    let mut f = StreamingFramer::new(24000, 24000, 60).expect("建立分帧器");
    assert!(f.push(&[]).expect("空分片").is_empty());
    assert!(f.finish().expect("空收尾").is_none());
}

mod native;
