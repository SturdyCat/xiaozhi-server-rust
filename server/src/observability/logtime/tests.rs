//! [`logtime`](super) 单元测试：统一时间戳格式形状。

use super::now_text;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;

#[test]
fn now_text_is_ymd_hms_local() {
    // 形状断言：YYYY-MM-DD HH:MM:SS（19 字符定长，数字与分隔符位次固定）。
    let s = now_text();
    assert_eq!(s.len(), 19, "长度: {s}");
    for (i, c) in s.chars().enumerate() {
        let ok = match i {
            4 | 7 => c == '-',
            10 => c == ' ',
            13 | 16 => c == ':',
            _ => c.is_ascii_digit(),
        };
        assert!(ok, "第 {i} 位字符不符：{s:?}");
    }
}

#[test]
fn local_datetime_timer_writes_stamp_into_writer() {
    let mut buf = String::new();
    super::LocalDatetime
        .format_time(&mut Writer::new(&mut buf))
        .expect("写入不应失败");
    assert_eq!(buf, now_text(), "计时器输出须与 now_text 同源同格式");
}
