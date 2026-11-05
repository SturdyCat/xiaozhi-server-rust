//! P5b typed lan-socket effect 与有界命令队列回归。

use super::super::effects::{
    enforce_permissions, Effect, LanSocketCommand, LanSocketQueue, Permissions,
};
use super::{ensure_fixture, host_with, ScriptedFetcher};
use std::sync::Arc;

#[test]
fn lan_socket_permission_is_explicit() {
    let effect = Effect::LanStatus {
        request_id: "r1".into(),
        device_handle: "opaque-handle".into(),
    };
    let denied = enforce_permissions(&Permissions::default(), vec![effect.clone()]);
    assert!(denied.is_err(), "未声明 lan-socket 必须拒绝");

    let allowed = Permissions::parse(&["lan-socket".into()]).expect("permission");
    assert!(enforce_permissions(&allowed, vec![effect]).is_ok());
}

#[test]
fn lan_queue_rejects_untrusted_shapes_and_is_bounded() {
    let queue = LanSocketQueue::new(1);
    queue
        .push(LanSocketCommand::SetDp {
            plugin_id: "test.plugin".into(),
            request_id: "r1".into(),
            device_handle: "opaque-handle".into(),
            dp: "1".into(),
            value_json: "true".into(),
        })
        .expect("valid command");
    let full = queue.push(LanSocketCommand::Status {
        plugin_id: "test.plugin".into(),
        request_id: "r2".into(),
        device_handle: "opaque-handle".into(),
    });
    assert!(full.is_err(), "队列满必须显式背压");

    let invalid_json = LanSocketQueue::default().push(LanSocketCommand::SetDp {
        plugin_id: "test.plugin".into(),
        request_id: "r3".into(),
        device_handle: "opaque-handle".into(),
        dp: "1".into(),
        value_json: "not-json".into(),
    });
    assert!(invalid_json.is_err(), "非法 value-json 必须 fail closed");

    let invalid_dp = LanSocketQueue::default().push(LanSocketCommand::SetDp {
        plugin_id: "test.plugin".into(),
        request_id: "r4".into(),
        device_handle: "opaque-handle".into(),
        dp: "raw-frame".into(),
        value_json: "true".into(),
    });
    assert!(invalid_dp.is_err(), "DP 标识不能承载原始报文");

    let invalid_plugin = LanSocketQueue::default().push(LanSocketCommand::Status {
        plugin_id: "bad\nplugin".into(),
        request_id: "r5".into(),
        device_handle: "opaque-handle".into(),
    });
    assert!(
        invalid_plugin.is_err(),
        "来源插件标识必须可安全用于 native 回灌路由"
    );
}

#[test]
fn hello_plugin_lan_effects_enter_native_queue() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");

    host.handle_event(
        "hello.official",
        "connector.tuya.status",
        r#"{"req_id":"r1"}"#,
    )
    .expect("status effect");
    host.handle_event(
        "hello.official",
        "connector.tuya.set-dp",
        r#"{"req_id":"r2"}"#,
    )
    .expect("set-dp effect");

    assert_eq!(host.pending_lan_commands(), 2);
    let commands = host.drain_lan_commands(8);
    assert!(matches!(
        &commands[0],
        LanSocketCommand::Status {
            plugin_id,
            request_id,
            device_handle
        } if plugin_id == "hello.official"
            && request_id == "r1" && device_handle == "fixture-handle"
    ));
    assert!(matches!(
        &commands[1],
        LanSocketCommand::SetDp {
            plugin_id,
            request_id,
            device_handle,
            dp,
            value_json
        } if plugin_id == "hello.official" && request_id == "r2"
            && device_handle == "fixture-handle"
            && dp == "1"
            && value_json == "true"
    ));
    assert_eq!(host.pending_lan_commands(), 0);
}

#[test]
fn lan_effect_without_queue_remains_explicitly_unsupported() {
    let effect = Effect::LanStatus {
        request_id: "r1".into(),
        device_handle: "opaque-handle".into(),
    };
    let kv = Arc::new(super::super::kv::KvStore::new());
    let fetcher: Arc<dyn super::super::effects::HttpFetcher> = Arc::new(ScriptedFetcher::new());
    let (errors, _) =
        super::super::effects::execute_effects("test.plugin", &[effect], &kv, &fetcher);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].to_string().contains("未绑定"));
}

#[allow(dead_code)]
fn _queue_type_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LanSocketQueue>();
}
