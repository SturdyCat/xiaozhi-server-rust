use super::*;
use serde_json::json;

#[test]
fn missing_workflow_uses_legacy_default() {
    let c = compile(&Value::Null).unwrap();
    assert_eq!(c.config.version, 1);
    assert_eq!(c.plan.len(), 5);
    assert_eq!(c.plan[0].plugin_id, "wakeup.greeting");
}
#[test]
fn default_has_one_model_gate_and_legacy_ids_are_marked_as_aliases() {
    let compiled = compile(&Value::Null).unwrap();
    let gates: Vec<_> = compiled
        .plan
        .iter()
        .filter(|b| safety_gate(&b.plugin_id))
        .collect();
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].plugin_id, "nlu.model");
    let metadata = projection(&Value::Null, "r".into());
    for id in ["command.gate", "nlu.rules", "skill.gate"] {
        let row = metadata["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .unwrap();
        assert_eq!(row["alias_of"], "nlu.model");
    }
}

#[test]
fn rejects_unknown_and_duplicate() {
    assert!(compile(
        &json!({"version":1,"bindings":[{"id":"x","plugin_id":"bad","position_id":"nlu"}]})
    )
    .is_err());
    let b = json!({"version":1,"bindings":[
      {"id":"a","plugin_id":"nlu.rules","position_id":"nlu","on_error":"stop"},
      {"id":"b","plugin_id":"nlu.rules","position_id":"nlu","on_error":"stop"}]});
    assert!(compile(&b).unwrap_err().contains("不可重复"));
}
#[test]
fn condition_projection_is_executable_shape() {
    let b = json!({"version":1,"bindings":[{"id":"x","plugin_id":"wakeup.greeting","position_id":"start","condition":{"kind":"device_kind","value":"echomuse"}}]});
    let c = compile(&b).unwrap();
    assert!(
        matches!(c.plan[0].condition, Condition::DeviceKind { ref value } if value == "echomuse")
    );
    let p = projection(&b, "r1".into());
    assert_eq!(p["revision"], "r1");
    assert_eq!(p["hot"], "next_session");
    assert_eq!(
        projection_with_mode(&b, "r1".into(), "aiui")["mode"],
        "aiui"
    );
}
#[test]
fn rejects_invalid_position_and_confidence() {
    let bad =
        json!({"version":1,"bindings":[{"id":"x","plugin_id":"nlu.rules","position_id":"vad"}]});
    assert!(compile(&bad).is_err());
    let bad = json!({"version":1,"bindings":[{"id":"x","plugin_id":"nlu.rules","position_id":"nlu","on_error":"stop","condition":{"kind":"confidence","min":0.8,"max":0.2}}]});
    assert!(compile(&bad).is_err());
}

#[test]
fn empty_explicit_workflow_is_valid_and_has_no_plan() {
    let c = compile(&json!({"version":1,"bindings":[]})).unwrap();
    assert!(c.plan.is_empty());
}

#[test]
fn rejects_unsupported_version_and_unknown_condition() {
    assert!(compile(&json!({"version":2,"bindings":[]}))
        .unwrap_err()
        .contains("version"));
    let b = json!({"version":1,"bindings":[{"id":"x","plugin_id":"wakeup.greeting","position_id":"start","condition":{"kind":"text_contains","text":"wake"}}]});
    assert!(compile(&b).is_err());
}

#[test]
fn disabled_binding_still_validates_policy_and_condition() {
    let bad_error = json!({"version":1,"bindings":[{"id":"x","plugin_id":"nlu.rules","position_id":"nlu","enabled":false,"on_error":"continue"}]});
    assert!(compile(&bad_error).is_err());
    let bad_condition = json!({"version":1,"bindings":[{"id":"x","plugin_id":"wakeup.greeting","position_id":"start","enabled":false,"condition":{"kind":"confidence","min":0.2}}]});
    assert!(compile(&bad_condition).is_err());
}

#[test]
fn duplicate_id_is_rejected_even_for_disabled_binding() {
    let b = json!({"version":1,"bindings":[
      {"id":"same","plugin_id":"wakeup.greeting","position_id":"start"},
      {"id":"same","plugin_id":"soul.profile","position_id":"link.nlu.llm","enabled":false}]});
    assert!(compile(&b).unwrap_err().contains("重复 binding"));
}

#[test]
fn duplicate_plugin_is_rejected_when_second_is_disabled() {
    let b = json!({"version":1,"bindings":[
      {"id":"a","plugin_id":"soul.profile","position_id":"link.nlu.llm"},
      {"id":"b","plugin_id":"soul.profile","position_id":"link.nlu.llm","enabled":false}]});
    assert!(compile(&b).unwrap_err().contains("不可重复"));
}

#[test]
fn rejects_empty_id_and_oversized_plan() {
    assert!(compile(
        &json!({"version":1,"bindings":[{"id":"","plugin_id":"soul.profile","position_id":"link.nlu.llm"}]})
    )
    .is_err());
    let bindings: Vec<Value> = (0..65).map(|i| json!({"id":format!("x{i}"),"plugin_id":"soul.profile","position_id":"link.nlu.llm","enabled":false})).collect();
    assert!(compile(&json!({"version":1,"bindings":bindings})).is_err());
}

#[test]
fn safety_gates_reject_continue_but_context_allows_it() {
    let gate = json!({"version":1,"bindings":[{"id":"x","plugin_id":"command.gate","position_id":"link.asr.nlu","on_error":"continue"}]});
    assert!(compile(&gate).is_err());
    let context = json!({"version":1,"bindings":[{"id":"x","plugin_id":"memory.recall","position_id":"link.nlu.llm","on_error":"continue"}]});
    assert!(compile(&context).is_ok());
}

#[test]
fn core_engine_descriptors_are_not_bindable() {
    let b = json!({"version":1,"bindings":[{"id":"x","plugin_id":"llm.openai","position_id":"link.nlu.llm"}]});
    assert!(compile(&b).unwrap_err().contains("核心引擎"));
}

#[test]
fn equal_position_and_order_uses_stable_binding_id_order() {
    let b = json!({"version":1,"bindings":[
      {"id":"z","plugin_id":"soul.profile","position_id":"link.nlu.llm","order":4},
      {"id":"a","plugin_id":"memory.recall","position_id":"link.nlu.llm","order":4}]});
    let c = compile(&b).unwrap();
    assert_eq!(
        c.plan
            .iter()
            .map(|x| x.plugin_id.as_str())
            .collect::<Vec<_>>(),
        vec!["memory.recall", "soul.profile"]
    );
}

#[test]
fn legacy_aliases_can_coexist_without_creating_multiple_recognizers() {
    let b = json!({"version":1,"bindings":[
      {"id":"a","plugin_id":"skill.gate","position_id":"nlu","on_error":"stop"},
      {"id":"b","plugin_id":"nlu.rules","position_id":"nlu","on_error":"stop"}]});
    assert_eq!(compile(&b).unwrap().plan.len(), 2);
}

#[test]
fn invalid_confidence_nan_and_out_of_range_are_rejected() {
    let high = json!({"version":1,"bindings":[{"id":"x","plugin_id":"nlu.rules","position_id":"nlu","on_error":"stop","condition":{"kind":"confidence","min":1.1}}]});
    assert!(compile(&high).is_err());
    let nan = json!({"version":1,"bindings":[{"id":"x","plugin_id":"nlu.rules","position_id":"nlu","on_error":"stop","condition":{"kind":"confidence","min":null}}]});
    assert!(compile(&nan).is_err());
}

#[test]
fn manifest_choices_are_all_compilable_and_safety_gates_only_offer_stop() {
    let metadata = projection(&Value::Null, "revision".into());
    for plugin in metadata["plugins"].as_array().unwrap() {
        let Some(position) = plugin["allowed_positions"].as_array().unwrap().first() else {
            continue;
        };
        let id = plugin["id"].as_str().unwrap();
        if safety_gate(id) {
            assert_eq!(plugin["on_error_options"], json!(["stop"]));
        }
        for kind in plugin["condition_kinds"].as_array().unwrap() {
            let condition = match kind.as_str().unwrap() {
                "always" => json!({"kind":"always"}),
                "text_contains" => {
                    json!({"kind":"text_contains","text":"hello","case_sensitive":false})
                }
                "device_kind" => json!({"kind":"device_kind","value":"echomuse"}),
                "confidence" => json!({"kind":"confidence","min":0.1,"max":0.9}),
                unknown => panic!("unsupported advertised condition {unknown}"),
            };
            for policy in plugin["on_error_options"].as_array().unwrap() {
                let value = json!({"version":1,"bindings":[{"id":"check","plugin_id":id,"position_id":position,"condition":condition,"on_error":policy}]});
                assert!(
                    compile(&value).is_ok(),
                    "advertised choice rejected: {value}"
                );
            }
        }
    }
}

#[test]
fn condition_values_are_validated_even_when_disabled() {
    for (plugin, position, condition, policy) in [
        (
            "command.gate",
            "link.asr.nlu",
            json!({"kind":"text_contains","text":"  "}),
            "stop",
        ),
        (
            "soul.profile",
            "link.nlu.llm",
            json!({"kind":"device_kind","value":"unknown"}),
            "continue",
        ),
    ] {
        for enabled in [false, true] {
            let value = json!({"version":1,"bindings":[{"id":"check","plugin_id":plugin,"position_id":position,"condition":condition,"on_error":policy,"enabled":enabled}]});
            assert!(compile(&value).is_err());
        }
    }
}

#[test]
fn invalid_projection_never_silently_returns_default_config() {
    let metadata = projection(&json!({"version":2,"bindings":[]}), "bad-revision".into());
    assert_eq!(metadata["valid"], false);
    assert_eq!(metadata["revision"], "bad-revision");
    assert!(metadata.get("config").is_none());
    assert!(metadata.get("plan").is_none());
}

#[test]
fn core_metadata_uses_real_registry_and_never_offers_bindings() {
    let metadata = projection(&Value::Null, "registry".into());
    let plugins = metadata["plugins"].as_array().unwrap();
    for id in [
        "asr.sensevoice",
        "vad.silero",
        "llm.openai",
        "tts.kokoro",
        "tts.xfyun",
        "fullchain.aiui",
    ] {
        let descriptor = crate::plugins::registry::find(id).unwrap();
        let row = plugins.iter().find(|p| p["id"] == id).unwrap();
        assert_eq!(row["label"], descriptor.display);
        assert_eq!(row["allowed_positions"], json!([]));
        assert_eq!(row["on_error_options"], json!([]));
        assert!(compile(&json!({"version":1,"bindings":[{"id":"invalid","plugin_id":id,"position_id":row["owner_position"]}]})).is_err());
    }
}

// ============================================================
// wasm 观察者绑定（P2d）
// ============================================================

fn observers() -> Vec<(String, String)> {
    vec![("hello.official".into(), "Hello 演示插件".into())]
}

fn observer_binding(position: &str, on_error: &str) -> Value {
    json!({"version":1,"bindings":[
        {"id":"b1","plugin_id":"hello.official","position_id":position,"enabled":true,"order":0,"condition":{"kind":"always"},"on_error":on_error}
    ]})
}

#[test]
fn observer_bindings_compile_on_hook_positions_only() {
    let obs = observers();
    // v1 钉死三个已接线 hook 位
    for pos in ["start", "link.nlu.llm", "end"] {
        let c = compile_with_observers(&observer_binding(pos, "continue"), &obs)
            .unwrap_or_else(|e| panic!("{pos} 应允许：{e}"));
        assert_eq!(c.plan[0].plugin_id, "hello.official");
    }
    // 非钩位拒绝（引擎位/门控位不是观察者能挂的）
    for pos in ["nlu", "link.asr.nlu", "asr", "llm"] {
        let err = compile_with_observers(&observer_binding(pos, "continue"), &obs).unwrap_err();
        assert!(err.contains("hook 位"), "{pos}：{err}");
    }
}

#[test]
fn observer_on_error_must_be_continue() {
    let err = compile_with_observers(&observer_binding("start", "stop"), &observers()).unwrap_err();
    assert!(err.contains("continue"), "{err}");
}

#[test]
fn observer_unknown_without_catalog_and_conditions_still_gate() {
    // 目录外 → 仍是未知插件（保存门不放松）；能力描述符仍拒绝
    let err = compile_with_observers(&observer_binding("start", "continue"), &[]).unwrap_err();
    assert!(err.contains("未知 plugin_id"), "{err}");
    let err = compile_with_observers(
        &json!({"version":1,"bindings":[
            {"id":"b1","plugin_id":"tts.kokoro","position_id":"start","enabled":true,"order":0,"condition":{"kind":"always"},"on_error":"continue"}
        ]}),
        &observers(),
    )
    .unwrap_err();
    assert!(err.contains("描述符"), "{err}");
    // 观察者不可重复声明；condition 沿用普通 hook 面（confidence 不可用）
    let dup = json!({"version":1,"bindings":[
        {"id":"b1","plugin_id":"hello.official","position_id":"start","enabled":true,"order":0,"condition":{"kind":"always"},"on_error":"continue"},
        {"id":"b2","plugin_id":"hello.official","position_id":"end","enabled":true,"order":1,"condition":{"kind":"always"},"on_error":"continue"}
    ]});
    assert!(compile_with_observers(&dup, &observers()).is_err());
    let conf = json!({"version":1,"bindings":[
        {"id":"b1","plugin_id":"hello.official","position_id":"start","enabled":true,"order":0,"condition":{"kind":"confidence","min":0.5},"on_error":"continue"}
    ]});
    assert!(compile_with_observers(&conf, &observers()).is_err());
}

#[test]
fn projection_lists_observers_with_marker() {
    let meta = projection_with_observers(
        &observer_binding("start", "continue"),
        "rev".into(),
        "local",
        &observers(),
    );
    let row = meta["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "hello.official")
        .expect("观察者应在节点目录");
    assert_eq!(row["observer"], true);
    assert_eq!(
        row["allowed_positions"],
        json!(["start", "link.nlu.llm", "end"])
    );
    assert_eq!(row["on_error_options"], json!(["continue"]));
}
