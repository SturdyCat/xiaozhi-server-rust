use super::*;
#[test]
fn media_identity_separates_codecs_and_revisions() {
    let pcm = AudioFormat::echo();
    let opus = AudioFormat::new(AudioCodec::Opus, 16000, 40);
    assert_ne!(pcm.identity(), opus.identity());
    let mut newer = pcm;
    newer.codec_version += 1;
    assert_ne!(pcm.identity(), newer.identity());
    assert_eq!(pcm.duration_us(1280), 40_000);
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
