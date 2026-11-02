use super::*;

#[test]
fn frame_types_and_codes_match_shared_contract() {
    // ready：任何 output 之前的第一帧，mode 一期恒 "pipe"。
    let ready = frame_json(&ConsoleFrame::Ready {
        device_id: "d1".into(),
        mode: frames::MODE_PIPE,
        raw_input: false,
        resize: false,
        insecure: true,
        notice: "n".into(),
    });
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["device_id"], "d1");
    assert_eq!(ready["mode"], "pipe");
    assert_eq!(ready["insecure"], true);
    assert_eq!(ready["notice"], "n");

    let output = frame_json(&ConsoleFrame::Output {
        text: "hi\n".into(),
    });
    assert_eq!(output["type"], "output");
    assert_eq!(output["text"], "hi\n");

    let notice = frame_json(&ConsoleFrame::Notice {
        code: frames::NoticeCode::OutputCapped,
        text: frames::COPY_CAPPED.into(),
    });
    assert_eq!(notice["type"], "notice");
    assert_eq!(notice["code"], "output-capped");
    assert_eq!(notice["text"], frames::COPY_CAPPED);

    // error.code 五个字面量 + 定值文案（§5.2 / §8.3）。
    for (code, literal, copy) in [
        (ErrorCode::Busy, "busy", frames::COPY_BUSY),
        (
            ErrorCode::InputTooLong,
            "input-too-long",
            frames::COPY_INPUT_TOO_LONG,
        ),
        (
            ErrorCode::RateLimited,
            "rate-limited",
            frames::COPY_RATE_LIMITED,
        ),
        (
            ErrorCode::DeviceOffline,
            "device-offline",
            frames::COPY_DEVICE_OFFLINE,
        ),
    ] {
        let json = frame_json(&ConsoleFrame::Error {
            code,
            error: code.message(""),
        });
        assert_eq!(json["type"], "error");
        assert_eq!(json["code"], literal);
        assert_eq!(json["error"], copy);
    }
    let internal = frame_json(&ConsoleFrame::Error {
        code: ErrorCode::Internal,
        error: ErrorCode::Internal.message("占槽失败"),
    });
    assert_eq!(internal["code"], "internal");
    assert_eq!(
        internal["error"],
        format!("{}占槽失败", frames::COPY_INTERNAL)
    );

    // closed.reason 八个共享字面量 + 仅审计用的 client-closed（`code()` 与 serde 同步）。
    let all = [
        (CloseReason::IdleTimeout, "idle-timeout"),
        (CloseReason::SessionLimit, "session-limit"),
        (CloseReason::DeviceOffline, "device-offline"),
        (CloseReason::Superseded, "superseded"),
        (CloseReason::DeviceShellLost, "device-shell-lost"),
        (CloseReason::Replaced, "replaced"),
        (CloseReason::ServerShutdown, "server-shutdown"),
        (CloseReason::Internal, "internal"),
        (CloseReason::ClientClosed, "client-closed"),
    ];
    for (reason, literal) in all {
        assert_eq!(reason.code(), literal);
        assert_eq!(
            serde_json::to_value(reason).unwrap().as_str(),
            Some(literal),
            "serde 与 code() 必须同字面量"
        );
        let json = frame_json(&ConsoleFrame::Closed {
            reason,
            detail: "d".into(),
            inputs: 3,
            bytes_out: 4096,
            secs: 7,
        });
        assert_eq!(json["type"], "closed");
        assert_eq!(json["reason"], literal);
        assert_eq!(json["detail"], "d");
        assert_eq!(json["inputs"], 3);
        assert_eq!(json["bytes_out"], 4096);
        assert_eq!(json["secs"], 7);
    }

    let pong = frame_json(&ConsoleFrame::Pong);
    assert_eq!(pong["type"], "pong");

    // shell_meta：与官方 dashboard 帧逐字一致（type 必须是 "shell_meta"，不是 kebab）。
    let meta = frame_json(&ConsoleFrame::ShellMeta { pty: true });
    assert_eq!(meta["type"], "shell_meta");
    assert_eq!(meta["pty"], true);
    let meta_pipe = frame_json(&ConsoleFrame::ShellMeta { pty: false });
    assert_eq!(meta_pipe["type"], "shell_meta");
    assert_eq!(meta_pipe["pty"], false);
}

#[tokio::test]
async fn pipe_ready_refuses_raw_resize_but_legacy_line_still_works() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "console-pipe-extension";
    let (shell_url, console_url, server) = loopback_server(id).await;
    let (_ctl, mut ctl_rx) = online(id, localhost(), true);
    let client = tokio::task::spawn_blocking(move || {
        let (mut ws, _) = tungstenite::connect(console_url.as_str()).unwrap();
        let ready = next_json(&mut ws);
        assert_eq!(ready["mode"], "pipe");
        assert_eq!(ready["raw_input"], false);
        assert_eq!(ready["resize"], false);
        assert_eq!(next_json(&mut ws)["pty"], false);
        for frame in [
            r#"{"type":"input","data":"BAD","raw":true}"#,
            r#"{"type":"resize","cols":80,"rows":24}"#,
        ] {
            ws.send(tungstenite::Message::Text(frame.into())).unwrap();
            assert_eq!(next_json(&mut ws)["code"], "unsupported-mode");
        }
        ws.send(tungstenite::Message::Text(
            r#"{"type":"resize","cols":0,"rows":24}"#.into(),
        ))
        .unwrap();
        assert_eq!(next_json(&mut ws)["code"], "invalid-resize");
        ws.send(tungstenite::Message::Text(
            r#"{"type":"input","data":"safe"}"#.into(),
        ))
        .unwrap();
        assert_eq!(next_json(&mut ws)["text"], "done");
        ws.close(None).unwrap();
        let _ = ws.read();
    });
    tokio::time::timeout(Duration::from_secs(5), ctl_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let device = tokio::task::spawn_blocking(move || {
        let (mut ws, _) = tungstenite::connect(format!("{shell_url}?pipe=1")).unwrap();
        match ws.read().unwrap() {
            tungstenite::Message::Binary(bytes) => assert_eq!(bytes.as_ref(), b"safe\n"),
            other => panic!("legacy line missing: {other:?}"),
        }
        ws.send(tungstenite::Message::Binary(b"done".to_vec().into()))
            .unwrap();
        let _ = ws.read();
    });
    tokio::time::timeout(Duration::from_secs(5), client)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), device)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(slot_owner(id), None);
    server.abort();
    em::hub_remove(id);
}
