//! `worker.rs` 纯函数单测：发现日志“状态有变化才升 INFO”的口径。

use super::scan_report_worthy;

#[test]
fn scan_report_worthy_pins_dedup_semantics() {
    // 首轮：没有上轮参照 → 打 INFO
    assert!(scan_report_worthy(None, 1));
    // 稳态：台数与上轮相同 → 只留 debug，不刷 INFO
    assert!(!scan_report_worthy(Some(1), 1));
    assert!(!scan_report_worthy(Some(0), 0));
    // 台数变化（新设备被发现 / 设备消失）→ 打 INFO
    assert!(scan_report_worthy(Some(1), 2));
    assert!(scan_report_worthy(Some(2), 1));
    assert!(scan_report_worthy(Some(0), 1));
}
