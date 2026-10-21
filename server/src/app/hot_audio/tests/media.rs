use super::*;
use crate::app::audio::media::{AudioCodec, AudioFormat};
#[test]
fn actual_codec_and_revision_separate_memory_and_disk_even_at_identical_geometry() {
    let old = HotAudioKey::from_session(&Config::default(), 16000, 40, "same speech");
    let opus = old
        .clone()
        .with_media_identity(AudioFormat::new(AudioCodec::Opus, 16000, 40));
    let pcm = old.clone().with_media_identity(AudioFormat::echo());
    let mut revision = AudioFormat::echo();
    revision.codec_version += 1;
    let newer = old.clone().with_media_identity(revision);
    let dir = tmp_dir();
    let store = HotAudioStore::new(Some(dir.clone()));
    assert!(store.put(&old, &frames(1)));
    assert!(store.put(&opus, &frames(2)));
    assert!(
        store.get(&pcm).is_none(),
        "legacy/Opus disk bytes must never become PCM"
    );
    assert!(store.put(&pcm, &frames(3)));
    for pair in [
        (old, pcm.clone()),
        (opus, pcm.clone()),
        (pcm.clone(), newer.clone()),
    ] {
        assert_ne!(pair.0.mem_key(), pair.1.mem_key());
        assert_ne!(pair.0.file_id(), pair.1.file_id());
    }
    assert_eq!(store.get(&pcm).unwrap(), frames(3));
    assert!(store.get(&newer).is_none());
    assert_eq!(HotAudioStore::new(Some(dir)).get(&pcm).unwrap(), frames(3));
}
