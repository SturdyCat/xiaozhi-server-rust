//! Restricted management contract and telemetry tests, without device commands.
use super::*;
use crate::device::echomuse as em;

#[tokio::test]
async fn shell_failures_have_distinct_non_success_http_contracts() {
    for (error, expected, code) in [
        (ShellError::NotFound, 404, "not_found"),
        (ShellError::Unapproved, 403, "unapproved"),
        (ShellError::Unsupported, 422, "unsupported_os"),
        (ShellError::MissingSpeaker, 422, "unsupported_capability"),
        (ShellError::Offline, 409, "offline"),
        (ShellError::Busy, 409, "busy"),
        (ShellError::Timeout, 504, "shell_timeout"),
        (ShellError::Transport, 502, "shell_transport"),
        (ShellError::OutputLimit, 502, "output_limit"),
        (ShellError::CommandFailed(7), 502, "command_failed"),
        // 文件下发（shell::stream_file）：三种失败都明确"没有干净结果"。
        (ShellError::NoDecoder, 502, "no_base64_decoder"),
        (ShellError::NoHashTool, 502, "no_hash_tool"),
        (ShellError::VerifyFailed, 502, "verify_failed"),
    ] {
        let response = error.response();
        assert_eq!(response.status().as_u16(), expected);
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], code);
        assert!(body["error"].is_string());
        assert!(body.get("queued").is_none());
        assert!(body.get("dispatched").is_none());
    }
}

#[test]
fn log_sources_are_closed_allowlist_and_scripts_are_fixed() {
    for source in [
        "server",
        "messages",
        "messages_last",
        "last_kmsg_prev",
        "supervisor",
        "net",
    ] {
        assert!(log_command(source).unwrap().starts_with("tail -c 49152 /"));
    }
    for source in [
        "/etc/shadow",
        "../messages",
        "messages;reboot",
        "messages\n",
        "",
    ] {
        assert!(log_command(source).is_none());
    }
    // 设备本机服务日志：原先只能插数据线开 console 才 cat 得到，现在走同一白名单远程读
    assert_eq!(log_command("server"), Some("tail -c 49152 /tmp/server.log"));
    assert_eq!(log_command("messages"), Some("tail -c 49152 /run/messages"));
    assert_eq!(
        log_command("messages_last"),
        Some("tail -c 49152 /data/emos/messages.last")
    );
    assert_eq!(
        log_command("last_kmsg_prev"),
        Some("tail -c 49152 /data/emos/last_kmsg.prev")
    );
    assert_eq!(log_command("net"), Some("tail -c 49152 /run/net.log"));
    assert!(REBOOT.contains("kill -TERM 1"));
    assert!(!REBOOT.contains("/system/bin/reboot"));
    assert!(
        !INFO.contains("cmdline"),
        "avoid exposing unrelated boot secrets"
    );
}

#[test]
fn reboot_requires_strict_explicit_confirmation() {
    assert!(confirmed(&json!({"confirm":true})));
    for body in [
        json!({}),
        json!(true),
        json!({"confirm":false}),
        json!({"confirm":"true"}),
        json!({"confirm":1}),
        json!({"confirm":true,"command":"other"}),
    ] {
        assert!(!confirmed(&body));
    }
}

#[test]
fn volume_is_raw_0_to_127_integer_and_never_arbitrary_command() {
    for level in [0, 1, 100, 127] {
        let out = volume_command(&json!({"level":level})).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&out).unwrap(),
            json!({"type":"volume_set","level":level})
        );
    }
    for body in [
        json!({"level":-1}),
        json!({"level":128}),
        json!({"level":0.5}),
        json!({"level":"10"}),
        json!({"level":true}),
        json!({"level":null}),
        json!({"level":10,"type":"reboot"}),
        json!({}),
    ] {
        assert!(volume_command(&body).is_err());
    }
}

#[test]
fn only_approved_registered_emos_passes_shell_policy() {
    let (reg, path) = em::tests::temp_registry("system-policy");
    assert_eq!(
        shell::approved_emos(&reg, "missing"),
        Err(ShellError::NotFound)
    );
    let mut record = em::tests::reg_load("system-policy", "test");
    record.base_os = Some("emos".into());
    reg.upsert(&record, 1);
    assert_eq!(
        shell::approved_emos(&reg, "system-policy"),
        Err(ShellError::Unapproved)
    );
    reg.approve("system-policy", None);
    assert!(shell::approved_emos(&reg, "system-policy").is_ok());
    for os in [None, Some("unknown"), Some("fireos"), Some("emOS")] {
        record.base_os = os.map(String::from);
        reg.upsert(&record, 2);
        assert_eq!(
            shell::approved_emos(&reg, "system-policy"),
            Err(ShellError::Unsupported)
        );
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn runtime_missing_remains_null_and_reports_are_observed_not_optimistic() {
    let live = runtime::LiveControl::new(None, true);
    let empty = live.payload("runtime");
    for key in [
        "stats",
        "listen_state",
        "volume_state",
        "mute_state",
        "updated_at",
    ] {
        assert!(empty[key].is_null());
    }
    live.update(&json!({"type":"stats","cpuPct":0,"wifiRssi":null,"aecRef":"hw","token":"secret","unknown":{"private":true}}));
    live.update(&json!({"type":"listen_state","state":"local","reason":"test"}));
    live.update(&json!({"type":"mute_state","muted":false}));
    live.update(&json!({"type":"volume_state","level":127}));
    let value = live.payload("runtime");
    assert_eq!(value["stats"]["cpuPct"], 0);
    assert!(value["stats"].get("memTotalMb").is_none());
    assert!(value["stats"].get("token").is_none());
    assert!(value["stats"].get("unknown").is_none());
    assert_eq!(value["listen_state"]["state"], "local");
    assert_eq!(value["volume_state"]["level"], 127);
    assert_eq!(value["mute_state"]["muted"], false);
    assert!(value["updated_at"].is_u64());
    // A command body is NOT a state report and must never alter observed state.
    live.update(&json!({"type":"volume_set","level":2}));
    live.update(&json!({"type":"volume_state","level":128}));
    live.update(&json!({"type":"listen_state","state":"invented"}));
    assert_eq!(live.payload("runtime"), value);
    let offline = runtime::offline_payload("runtime");
    assert_eq!(offline["online"], false);
    assert!(offline["volume_state"].is_null());
}

#[test]
fn runtime_telemetry_is_bounded_and_not_merged_into_stale_reports() {
    let live = runtime::LiveControl::new(None, true);
    live.update(&json!({"type":"stats","cpuPct":3,"memTotalMb":10}));
    live.update(&json!({"type":"stats","cpuPct":4}));
    assert!(live.payload("id")["stats"].get("memTotalMb").is_none());
    let before = live.payload("id");
    live.update(&json!({"type":"stats","wifiSsid":"x".repeat(runtime::MAX_TELEMETRY_BYTES)}));
    assert_eq!(live.payload("id"), before);
    live.update(&json!({"type":"stats","wifiSsid":"x".repeat(257),"cpuPct":5}));
    assert!(live.payload("id")["stats"].get("wifiSsid").is_none());
    assert_eq!(live.payload("id")["stats"]["cpuPct"], 5);
}

#[test]
fn settings_use_real_go_types_ranges_and_migrate_legacy_push() {
    for body in [
        json!({"owwOnDevice":"on","wakeSoundLevel":"medium","wakeMic":6,"owwThreshold":0.05}),
        json!({"owwOnDevice":"shadow","wakeSoundLevel":"quiet","wakeMic":0,"owwThreshold":1}),
    ] {
        assert!(admin::filter_settings(&body).1.is_none());
    }
    for body in [
        json!({"owwOnDevice":true}),
        json!({"owwOnDevice":"enabled"}),
        json!({"wakeSoundLevel":50}),
        json!({"wakeSoundLevel":"max"}),
        json!({"wakeMic":6.1}),
        json!({"wakeMic":7}),
        json!({"wakeMic":-1}),
        json!({"owwThreshold":0}),
        json!({"owwThreshold":1.1}),
    ] {
        assert!(
            admin::filter_settings(&body).1.is_some(),
            "accepted invalid {body}"
        );
    }
    let pushed = em::config_push_with(&json!({"owwOnDevice":true,"wakeSoundLevel":80,"wakeMic":9}));
    let wire: Value = serde_json::from_str(&pushed[0]).unwrap();
    assert_eq!(wire["owwOnDevice"], "on");
    assert_eq!(wire["wakeSoundLevel"], "medium");
    assert!(wire.get("wakeMic").is_none());
}

#[test]
fn partial_settings_merge_and_legacy_registry_load() {
    let (reg, path) = em::tests::temp_registry("settings-merge");
    reg.upsert(&em::tests::reg_load("settings-merge", "test"), 1);
    reg.set_settings(
        "settings-merge",
        [
            ("owwOnDevice".into(), json!(false)),
            ("wakeSoundLevel".into(), json!(40)),
        ]
        .into_iter()
        .collect(),
    );
    reg.set_settings(
        "settings-merge",
        [("wakeWordEnabled".into(), json!(false))]
            .into_iter()
            .collect(),
    );
    assert_eq!(reg.get("settings-merge").unwrap().settings.len(), 3);
    let loaded = registry::Registry::load(path.clone());
    let settings = loaded.get("settings-merge").unwrap().settings;
    assert_eq!(settings["owwOnDevice"], "off");
    assert_eq!(settings["wakeSoundLevel"], "medium");
    assert_eq!(settings["wakeWordEnabled"], false);
    let pushed = em::config_push_with(&Value::Object(settings.into_iter().collect()));
    let wire: Value = serde_json::from_str(&pushed[0]).unwrap();
    assert_eq!(
        wire["owwOnDevice"], "off",
        "merged snapshot must not reset to baseline on"
    );
    let _ = std::fs::remove_file(path);
}

/// `/runtime` 的 `link` 块三态：在线已认证 / 在线未认证 / 离线（两字段不可知）。
/// 纯附加字段：旧调用方（`esp_home`、`payload()`）一个字段都没少。
#[test]
fn runtime_link_reports_authentication_and_token_policy() {
    let authed = runtime::LiveControl::new(Some("192.0.2.9".parse().unwrap()), true);
    let body = authed.payload_with_link("d1", true);
    assert_eq!(body["link"]["authenticated"], true);
    assert_eq!(body["link"]["token_required"], true);
    assert_eq!(body["online"], true);

    let lan = runtime::LiveControl::new(None, false).payload_with_link("d1", false);
    assert_eq!(lan["link"]["authenticated"], false);
    assert_eq!(lan["link"]["token_required"], false);

    // 离线：link 明确为 null 且键存在（与"没这个概念"区分）。
    let offline = runtime::offline_payload("d1");
    assert!(offline.as_object().unwrap().contains_key("link"));
    assert!(offline["link"].is_null());
    assert_eq!(offline["online"], false);

    // `payload()` 保持原样（不含 link）：`esp_home` 与既有断言据此不变。
    assert!(authed.payload("d1").get("link").is_none());
}

/// settings 白名单边界：官方文档曾误列的 8 个键必须继续被拒，`vadChannel` 永不入白名单。
#[test]
fn settings_whitelist_keeps_rejecting_documented_but_nonexistent_keys() {
    for key in [
        "vadChannel",
        "ledScene",
        "ledListenColor",
        "ledThinkColor",
        "meterAttack",
        "nsAsr",
        "owwSpeexNs",
        "buttonSingleTapEvent",
    ] {
        let (_, error) = admin::filter_settings(&json!({key: false}));
        assert!(
            error.is_some_and(|e| e.contains(key)),
            "{key} 必须被白名单拒绝"
        );
    }
}

/// 白名单的**正例与计数**：恰好 40 键（设备线缆真键集），`sendspinUnpaired` 已收，
/// `consolePassword`/`consoleTimeoutMin` 仍刻意不收（那是 emOS **USB 串口 console**
/// 的 init 凭据/超时，与网页控制台无关）。多收一个或少收一个都会在这里炸。
#[test]
fn settings_whitelist_is_exactly_40_keys_with_sendspin_unpaired() {
    assert_eq!(
        admin::SETTINGS_KEYS.len(),
        40,
        "白名单固定 40 键；增删必须同步 admin.rs 注释、docs/echomuse.md 与本断言"
    );
    // 官方 Stage07 的「已配对另一台 MA」开关：设备线缆真有（config.go:601 `*bool`）。
    assert!(
        admin::filter_settings(&json!({"sendspinUnpaired": true}))
            .1
            .is_none(),
        "sendspinUnpaired 必须被接受"
    );
    // 灯环真正在设备线缆里的那个键（json 任意对象）。
    assert!(
        admin::filter_settings(&json!({"listeningAnim": {"name": "listen"}}))
            .1
            .is_none(),
        "listeningAnim 必须被接受"
    );
    // 仍刻意不收的两个（真键但与本页控制台无关，别被 "console" 这个词误导）。
    for key in ["consolePassword", "consoleTimeoutMin"] {
        assert!(
            admin::filter_settings(&json!({key: true})).1.is_some(),
            "{key} 是 P2 项（USB 串口 console），不得进白名单"
        );
    }
}
