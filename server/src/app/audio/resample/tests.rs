//! `server/src/app/audio/resample.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::StreamingResampler;

fn ramp(n: usize) -> Vec<f32> {
    (0..n).map(|i| i as f32 / n as f32).collect()
}

/// 分片推送与整段推送结果一致（相位连续，无边界跳变）。
#[test]
fn streamed_chunks_match_single_push() {
    let input = ramp(1000);
    let mut whole = StreamingResampler::new(16000, 24000);
    let expect = whole.push(&input);

    let mut streamed = StreamingResampler::new(16000, 24000);
    let mut got = Vec::new();
    for chunk in input.chunks(160) {
        got.extend(streamed.push(chunk));
    }
    assert_eq!(got.len(), expect.len());
    for (a, b) in got.iter().zip(expect.iter()) {
        assert!((a - b).abs() < 1e-6, "分片结果与整段不一致: {a} vs {b}");
    }
}

/// 相同采样率透传。
#[test]
fn passthrough_when_same_rate() {
    let input = ramp(100);
    let mut r = StreamingResampler::new(24000, 24000);
    assert_eq!(r.push(&input), input);
}

/// 输出样本数 ≈ 输入 × ratio（允许 ±2）。
#[test]
fn output_length_tracks_ratio() {
    let input = ramp(1600); // 100ms @16k
    let mut r = StreamingResampler::new(16000, 24000);
    let out = r.push(&input);
    assert!((out.len() as i64 - 2400).abs() <= 2, "got {}", out.len());
}

/// 压缩逻辑不丢数据：长流分片推进后 tail 有界且结果连续。
#[test]
fn tail_bounded_and_continuous() {
    let mut r = StreamingResampler::new(16000, 24000);
    let mut total = 0usize;
    for _ in 0..200 {
        total += r.push(&ramp(160)).len();
    }
    // 200×160=32000 输入样本 → 48000 输出（±2）
    assert!((total as i64 - 48000).abs() <= 2, "got {total}");
    assert!(r.tail.len() < 8, "tail 未收敛: {}", r.tail.len());
}

/// Echo 下行必经的 24k → 48k：整数倍 2× 升采样，长流样本数恒 ≈ 2× 输入（不漂移）。
///
/// 这条路径是收敛后**唯一**的重采样边（ESP 24k 走透传），所以它必须精确：
/// 一旦样本数漂移，40ms/3840B 的定长分帧会被 media 层按块长校验拒收，表现为静音。
#[test]
fn echo_24k_to_48k_doubles_without_drift() {
    let mut r = StreamingResampler::new(24_000, 48_000);
    let mut total = 0usize;
    let fed = 100 * 960; // 100 × 40ms @24k
    for _ in 0..100 {
        total += r.push(&ramp(960)).len();
    }
    assert!(
        (total as i64 - 2 * fed as i64).abs() <= 2,
        "2× 升采样样本数漂移：got {total}，期望 {}",
        2 * fed
    );
}

/// 2× 升采样的取值：原样点 + 中点（线性插值），无相位偏移。
///
/// 注意输出永远比理论值少 1~2 个样本：插值需要"右邻样本"，流末那个位置要等下一片才成立
/// （由分帧器的 carry/尾帧补零兜底）。
#[test]
fn double_rate_interpolates_midpoints() {
    let mut r = StreamingResampler::new(24_000, 48_000);
    let out = r.push(&[0.0, 1.0, 2.0]);
    assert_eq!(out.len(), 4);
    assert!((out[0] - 0.0).abs() < 1e-6);
    assert!((out[1] - 0.5).abs() < 1e-6, "中点必须是两样本均值");
    assert!((out[2] - 1.0).abs() < 1e-6, "原样本位置保持原值");
    assert!((out[3] - 1.5).abs() < 1e-6);
}
