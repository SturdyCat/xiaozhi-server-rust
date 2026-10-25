use super::*;
#[test]
fn native_pcm_is_sample_exact_and_requires_no_opus() {
    let chunk = AudioChunk::new(AudioFormat::echo_mic(), vec![0, 128, 0, 0, 255, 127]);
    assert_eq!(
        native_pcm(&chunk).unwrap(),
        vec![-1.0, 0.0, 32767.0 / 32768.0]
    );
    assert!(!needs_opus(chunk.format.codec));
    assert!(native_pcm(&AudioChunk::new(AudioFormat::echo_mic(), vec![0])).is_none());
}

#[test]
fn device_vad_accumulates_chunks_into_one_utterance() {
    // 设备 VAD 直通：多块 160ms PCM 必须拼成**一整段**——逐块成段会让 ASR
    // 对每个 160ms 碎片各跑一次（太短→空→静默跳过），真机表现为"说了话无反应"。
    let mut pending: Vec<Vec<f32>> = Vec::new();
    accumulate_device_pcm(&mut pending, &[1.0, 2.0]);
    accumulate_device_pcm(&mut pending, &[3.0]);
    accumulate_device_pcm(&mut pending, &[4.0, 5.0, 6.0]);
    assert_eq!(pending, vec![vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]]);
    // 收口后（pending 被 take）下一轮从新段开始。
    pending.clear();
    accumulate_device_pcm(&mut pending, &[7.0]);
    assert_eq!(pending, vec![vec![7.0]]);
}
