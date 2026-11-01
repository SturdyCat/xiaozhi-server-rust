use super::*;

#[test]
fn local_melody_is_short_bounded_non_silent_and_fades_at_note_edges() {
    let pcm = melody();
    assert_eq!(pcm.len(), SAMPLE_RATE as usize * 720 / 1000);
    assert!(pcm
        .iter()
        .all(|sample| sample.is_finite() && sample.abs() <= 0.22));
    assert!(pcm.iter().any(|sample| sample.abs() > 0.1));
    for note in pcm.chunks(SAMPLE_RATE as usize * 120 / 1000) {
        assert_eq!(note[0], 0.0);
        assert!(note[note.len() - SAMPLE_RATE as usize * 20 / 1000..]
            .iter()
            .all(|s| *s == 0.0));
    }
}
