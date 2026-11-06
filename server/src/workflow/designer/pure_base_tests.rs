use super::*;
use serde_json::json;

#[test]
fn missing_workflow_is_empty_in_pure_base() {
    let compiled = compile(&Value::Null).unwrap();
    assert!(compiled.plan.is_empty());
    assert!(compiled.config.bindings.is_empty());
}

#[test]
fn static_business_bindings_are_not_available() {
    for (id, position) in [
        ("wakeup.greeting", "start"),
        ("memory.recall", "link.nlu.llm"),
        ("nlu.model", "nlu"),
    ] {
        let error = compile(&json!({
            "version": 1,
            "bindings": [{"id": "business", "plugin_id": id, "position_id": position}]
        }))
        .unwrap_err();
        assert!(
            error.contains("未知 plugin_id") || error.contains("描述符"),
            "{id}: {error}"
        );
    }
}

#[test]
fn projection_contains_core_and_runtime_observer_entries_only() {
    let projection = projection(&Value::Null, "pure-base".into());
    let plugins = projection["plugins"].as_array().unwrap();
    assert!(plugins.iter().all(|plugin| {
        !matches!(
            plugin["id"].as_str(),
            Some(
                "wakeup.greeting"
                    | "soul.profile"
                    | "memory.recall"
                    | "memory.writeback"
                    | "nlu.model"
            )
        )
    }));
    assert!(plugins.iter().any(|plugin| plugin["id"] == "llm.openai"));

    let observer = projection_with_observers(
        &Value::Null,
        "pure-base-observer".into(),
        "local",
        &[("example.observer".into(), "示例观察者".into())],
    );
    assert!(observer["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .any(|plugin| plugin["id"] == "example.observer"));
}

#[test]
fn explicit_empty_workflow_remains_valid() {
    let compiled = compile(&json!({"version": 1, "bindings": []})).unwrap();
    assert!(compiled.plan.is_empty());
}

#[test]
fn runtime_observers_can_bind_hook_positions() {
    let config = json!({
        "version": 1,
        "bindings": [{
            "id": "observer-binding",
            "plugin_id": "example.observer",
            "position_id": "start",
            "condition": {"kind": "always"},
            "on_error": "continue"
        }]
    });
    let compiled =
        compile_with_observers(&config, &[("example.observer".into(), "示例观察者".into())])
            .unwrap();
    assert_eq!(compiled.plan[0].plugin_id, "example.observer");
}

#[test]
fn core_engine_descriptors_remain_non_bindable() {
    let error = compile(&json!({
        "version": 1,
        "bindings": [{"id": "core", "plugin_id": "llm.openai", "position_id": "llm"}]
    }))
    .unwrap_err();
    assert!(
        error.contains("核心引擎") || error.contains("描述符"),
        "{error}"
    );
}
