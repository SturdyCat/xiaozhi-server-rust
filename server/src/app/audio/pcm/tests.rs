//! `server/src/app/audio/pcm.rs` 的单元测试。
//!
//! 自 `app/session.rs` 的 `f32_to_i16_16k` 下移时一并迁移（行为不变的搬移，
//! AGENTS.md §5.10）；断言口径保持不变。

use super::f32_to_i16_16k;

#[test]
fn passthrough_at_16k() {
    let out = f32_to_i16_16k(&[0.0, 0.5, -0.5, 1.0, -1.0], 16_000);
    assert_eq!(out, vec![0, 16383, -16383, 32767, -32767]);
}

#[test]
fn downsample_from_48k_linear_interpolation() {
    // 48k → 16k：每 3 个样本取 1 个（插值点恰好落在整数索引上）
    let pcm: Vec<f32> = (0..12).map(|i| i as f32 / 12.0).collect();
    let out = f32_to_i16_16k(&pcm, 48_000);
    assert_eq!(out.len(), 4, "48k→16k 应抽取 1/3 样本");
    assert_eq!(out[0], 0);
    // 插值点落在整数索引 3、6、9 → 值即原样本（钳位后 × 32767）
    assert_eq!(out[1], (pcm[3] * 32767.0) as i16);
}

#[test]
fn clamps_out_of_range() {
    let out = f32_to_i16_16k(&[2.0, -2.0], 16_000);
    assert_eq!(out, vec![32767, -32767], "超界样本必须钳位到 i16 范围");
}

#[test]
fn empty_input_is_empty() {
    assert!(f32_to_i16_16k(&[], 16_000).is_empty());
    assert!(f32_to_i16_16k(&[], 48_000).is_empty());
}
