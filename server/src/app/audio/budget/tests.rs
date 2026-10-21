use super::*;
#[test]
fn duration_and_bytes_are_independent_and_returned_on_drop() {
    let b = AudioBudget::new(40, 1280);
    let permit = b.reserve(40_000, 1280).unwrap();
    assert!(b.reserve(1, 0).is_none());
    assert!(b.reserve(0, 1).is_none());
    drop(permit);
    assert_eq!(b.usage(), (0, 0));
    assert!(b.reserve(40_001, 2).is_none());
    assert!(b.reserve(1, 1281).is_none());
}
