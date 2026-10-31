use super::*;
use frames::{Incoming, InputAction, RawBudget};

#[test]
fn raw_preserves_controls_multiline_unicode_and_no_added_newline() {
    let now = Instant::now();
    let mut raw = RawBudget::new(now);
    let data = " 你好\r\n\u{1b}[A\u{3}\0";
    let json = serde_json::json!({"type":"input","data":data,"raw":true});
    assert_eq!(
        frames::parse_incoming(&json.to_string()),
        Incoming::RawInput(data.into())
    );
    let mut expected = vec![0];
    expected.extend_from_slice(data.as_bytes());
    assert_eq!(
        frames::plan_raw(data, &mut raw, now, true),
        InputAction::Write(expected)
    );
    for raw_field in [serde_json::json!({}), serde_json::json!({"raw":false})] {
        let mut line = serde_json::json!({"type":"input","data":"ls\r\n"});
        line.as_object_mut()
            .unwrap()
            .extend(raw_field.as_object().unwrap().clone());
        assert_eq!(
            frames::parse_incoming(&line.to_string()),
            Incoming::Input("ls".into())
        );
        assert_eq!(
            frames::plan_line(&mut TokenBucket::new(10), now, "ls", true),
            InputAction::Write(b"\0ls\n".to_vec())
        );
        assert_eq!(
            frames::plan_line(&mut TokenBucket::new(10), now, "ls", false),
            InputAction::Write(b"ls\n".to_vec())
        );
    }
    assert_eq!(
        frames::parse_incoming(r#"{"type":"input","data":"x","raw":1}"#),
        Incoming::Unknown
    );
}

#[test]
fn resize_bounds_and_big_endian_encoding() {
    let now = Instant::now();
    let mut budget = RawBudget::new(now);
    assert_eq!(
        frames::parse_incoming(r#"{"type":"resize","cols":500,"rows":200}"#),
        Incoming::Resize(500, 200)
    );
    assert_eq!(
        frames::plan_resize(500, 200, &mut budget, now, true),
        InputAction::Write(vec![1, 1, 244, 0, 200])
    );
    for (cols, rows) in [
        (serde_json::json!(0), serde_json::json!(1)),
        (serde_json::json!(501), serde_json::json!(1)),
        (serde_json::json!(1), serde_json::json!(201)),
        (serde_json::json!(-1), serde_json::json!(1)),
        (serde_json::json!(1.0), serde_json::json!(1)),
        (serde_json::json!("80"), serde_json::json!(1)),
        (serde_json::json!(65536), serde_json::json!(1)),
        (serde_json::Value::Null, serde_json::json!(1)),
    ] {
        assert_eq!(
            frames::parse_incoming(
                &serde_json::json!({"type":"resize","cols":cols,"rows":rows}).to_string()
            ),
            Incoming::InvalidResize
        );
    }
    assert_eq!(
        frames::plan_resize(0, 1, &mut budget, now, true),
        InputAction::Reject(ErrorCode::InvalidResize)
    );
}

#[test]
fn pipe_rejects_extensions_without_unsafe_wire_bytes() {
    let now = Instant::now();
    let mut raw = RawBudget::new(now);
    assert_eq!(
        frames::plan_raw("reboot", &mut raw, now, false),
        InputAction::Reject(ErrorCode::UnsupportedMode)
    );
    assert_eq!(
        frames::plan_resize(80, 24, &mut raw, now, false),
        InputAction::Reject(ErrorCode::UnsupportedMode)
    );
}

#[test]
fn raw_events_and_bytes_are_bounded_independently_of_lines() {
    let now = Instant::now();
    let mut budget = RawBudget::new(now);
    for _ in 0..100 {
        assert!(matches!(
            frames::plan_raw("a", &mut budget, now, true),
            InputAction::Write(_)
        ));
    }
    assert_eq!(
        frames::plan_raw("a", &mut budget, now, true),
        InputAction::Reject(ErrorCode::RateLimited)
    );
    assert!(matches!(
        frames::plan_raw(
            "a",
            &mut budget,
            now + std::time::Duration::from_secs(1),
            true
        ),
        InputAction::Write(_)
    ));
    let mut budget = RawBudget::new(now);
    assert_eq!(
        frames::plan_raw(&"x".repeat(4097), &mut budget, now, true),
        InputAction::Reject(ErrorCode::InputTooLong)
    );
    for _ in 0..16 {
        assert!(matches!(
            frames::plan_raw(&"x".repeat(4096), &mut budget, now, true),
            InputAction::Write(_)
        ));
    }
    assert_eq!(
        frames::plan_raw("x", &mut budget, now, true),
        InputAction::Reject(ErrorCode::RateLimited)
    );
    assert!(matches!(
        frames::plan_line(&mut TokenBucket::new(10), now, "ls", true),
        InputAction::Write(_)
    ));
}

#[test]
fn ready_capabilities_and_json_output_preserve_escape_sequences() {
    for pty in [false, true] {
        let ready = frames::encode(&ConsoleFrame::Ready {
            device_id: "d".into(),
            mode: if pty {
                frames::MODE_PTY
            } else {
                frames::MODE_PIPE
            },
            raw_input: pty,
            resize: pty,
            insecure: false,
            notice: "".into(),
        });
        let ready: serde_json::Value = serde_json::from_str(&ready).unwrap();
        assert_eq!(ready["raw_input"], pty);
        assert_eq!(ready["resize"], pty);
    }
    let mut carry = Utf8Carry::new();
    let source = "\u{1b}[31m你好\u{1b}[0m\r\n\u{7}";
    let mut result = String::new();
    for byte in source.as_bytes() {
        let part = carry.push(&[*byte]);
        let encoded = frames::encode(&ConsoleFrame::Output { text: part });
        let json: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        result.push_str(json["text"].as_str().unwrap());
    }
    assert_eq!(result, source);
}
