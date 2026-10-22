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
