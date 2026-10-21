use super::*;
#[test]
fn media_identity_separates_codecs_and_revisions() {
    let pcm = AudioFormat::echo_mic();
    let opus = AudioFormat::new(AudioCodec::Opus, 16000, 40);
    assert_ne!(pcm.identity(), opus.identity());
    let mut newer = pcm;
    newer.codec_version += 1;
    assert_ne!(pcm.identity(), newer.identity());
    assert_eq!(pcm.duration_us(1280), 40_000);
    // Echo 下行线缆是 48 kHz：同样 40 ms 的一整块是 3840 字节。
    assert_eq!(AudioFormat::echo_speaker().duration_us(3840), 40_000);
}
#[test]
fn old_connection_stamps_never_become_current_again() {
    let generation = Arc::new(AtomicU64::new(1));
    let stamp = AudioEpoch::capture(&generation, 7);
    assert!(stamp.is_current());
    generation.fetch_add(1, Ordering::AcqRel);
    assert!(!stamp.is_current());
    assert_eq!(stamp.turn, 7);
}
