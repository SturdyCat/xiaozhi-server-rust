use super::*;
use crate::media::media::AudioFormat;
#[test]
fn resampled_native_pcm_is_byte_identical_across_arbitrary_provider_chunk_boundaries() {
    let input: Vec<f32> = (0..4501)
        .map(|i| ((i % 1000) as f32 - 500.0) / 500.0)
        .collect();
    let mut whole = StreamingFramer::new_format(24000, AudioFormat::echo_speaker()).unwrap();
    let mut pieces = StreamingFramer::new_format(24000, AudioFormat::echo_speaker()).unwrap();
    let mut a = whole.push(&input).unwrap();
    if let Some(tail) = whole.finish().unwrap() {
        a.push(tail);
    }
    let mut b = Vec::new();
    for part in input.chunks(137) {
        b.extend(pieces.push(part).unwrap());
    }
    if let Some(tail) = pieces.finish().unwrap() {
        b.push(tail);
    }
    assert_eq!(a, b);
    assert!(a.len() > 3);
    // 48 kHz × 40 ms × 2 字节 = 3840：Echo speaker 每块必须是这个长度。
    assert!(a.iter().all(|f| f.len() == 3840));
}
#[test]
fn native_output_is_s16le_with_exact_sample_values_and_zero_padded_tail() {
    // 同采样率（16k→16k）透传，才能断言样本值逐点精确。
    let mut f = StreamingFramer::new_format(16000, AudioFormat::echo_mic()).unwrap();
    assert!(f.push(&[-2.0, -1.0, 0.0, 1.0, 2.0]).unwrap().is_empty());
    let bytes = f.finish().unwrap().unwrap();
    let pcm: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|s| i16::from_le_bytes([s[0], s[1]]))
        .collect();
    assert_eq!(&pcm[..5], &[-32767, -32767, 0, 32767, 32767]);
    assert!(pcm[5..].iter().all(|s| *s == 0));
    assert!(f.finish().unwrap().is_none());
}
