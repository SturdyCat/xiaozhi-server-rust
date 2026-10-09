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
