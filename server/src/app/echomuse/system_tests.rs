//! Restricted management contract and telemetry tests, without device commands.
use super::*;
use crate::app::echomuse as em;

#[tokio::test]
async fn shell_failures_have_distinct_non_success_http_contracts() {
    for (error, expected, code) in [
        (ShellError::NotFound, 404, "not_found"),
        (ShellError::Unapproved, 403, "unapproved"),
        (ShellError::Unsupported, 422, "unsupported_os"),
        (ShellError::MissingSpeaker, 422, "unsupported_capability"),
        (ShellError::Offline, 409, "offline"),
        (ShellError::Busy, 409, "busy"),
        (ShellError::MissingToken, 503, "token_required"),
        (ShellError::Timeout, 504, "shell_timeout"),
        (ShellError::Transport, 502, "shell_transport"),
        (ShellError::OutputLimit, 502, "output_limit"),
        (ShellError::CommandFailed(7), 502, "command_failed"),
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
    for source in ["messages", "messages_last", "last_kmsg_prev", "supervisor"] {
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
    assert_eq!(log_command("messages"), Some("tail -c 49152 /run/messages"));
    assert_eq!(
        log_command("messages_last"),
        Some("tail -c 49152 /data/emos/messages.last")
    );
    assert_eq!(
        log_command("last_kmsg_prev"),
        Some("tail -c 49152 /data/emos/last_kmsg.prev")
    );
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
