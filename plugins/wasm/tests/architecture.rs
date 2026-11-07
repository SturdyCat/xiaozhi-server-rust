//! P8 architecture guards: WASM bridges stay above native data-plane modules.

#[test]
fn bridge_layer_has_no_native_data_plane_imports() {
    let bridge = include_str!("../bridge.rs");
    let policies = include_str!("../bridge/policies.rs");
    let nlu = include_str!("../bridge/nlu.rs");
    for (name, source) in [
        ("bridge.rs", bridge),
        ("policies.rs", policies),
        ("nlu.rs", nlu),
    ] {
        for forbidden in [
            "crate::device::",
            "crate::gateway::",
            "crate::workflow::",
            "crate::runtime::",
            "std::net::TcpStream",
            "reqwest::",
        ] {
            assert!(
                !source.contains(forbidden),
                "{name} must not import native data-plane symbol {forbidden}"
            );
        }
    }
}

#[test]
fn policy_sources_keep_native_route_contract_explicit() {
    let policies = include_str!("../bridge/policies.rs");
    let nlu = include_str!("../bridge/nlu.rs");
    assert!(policies.contains("route != \"native\""));
    assert!(nlu.contains("route != \"native\""));
}

#[test]
fn native_data_plane_ownership_markers_remain_present_before_cleanup() {
    let tuya = include_str!("../../../server/src/device/tuya/wasm.rs");
    let echomuse = include_str!("../../../server/src/device/echomuse/runtime.rs");
    let memory = include_str!("../../memory/mod.rs");
    assert!(tuya.contains("drain_wasm_commands"));
    assert!(tuya.contains("set_dp"));
    assert!(echomuse.contains("hub_remove_session") || echomuse.contains("session"));
    assert!(memory.contains("MemoryProvider") || memory.contains("GraphMemory"));
}

#[test]
fn runtime_keeps_artifact_integrity_gate_before_wasm_load() {
    let runtime = include_str!("../../../server/src/runtime/mod.rs");
    assert!(runtime.contains("artifact_matches"));
    assert!(runtime.contains("ArtifactDir::digest"));
    assert!(runtime.contains("continue;"));
}

#[test]
fn registry_selection_keeps_wasm_policy_explicit_without_native_fallback() {
    let selection = include_str!("../../../plugins/registry/impls/selection.rs");
    for predicate in [
        "cfg.tuya.engine_id() == \"wasm-policy\"",
        "cfg.echomuse.engine_id() == \"wasm-policy\"",
        "cfg.vad.engine_id() == \"wasm-policy\"",
        "cfg.tts.engine_id() == \"wasm-policy\"",
        "cfg.llm.engine_id() == \"wasm-policy\"",
    ] {
        assert!(
            selection.contains(predicate),
            "missing explicit selection predicate: {predicate}"
        );
    }
}

#[test]
fn device_capability_registry_stays_native_and_wasm_optional() {
    let registry = include_str!("../../../server/src/device/capabilities/registry.rs");
    assert!(!registry.contains("plugins::wasm"));
    assert!(!registry.contains("WasmHost"));
    assert!(registry.contains("DeviceCapabilityAdapter"));
}

#[test]
fn native_hot_paths_do_not_dispatch_wasm() {
    for (name, source) in [
        (
            "echomuse/runtime.rs",
            include_str!("../../../server/src/device/echomuse/runtime.rs"),
        ),
        (
            "echomuse/transport.rs",
            include_str!("../../../server/src/device/echomuse/transport.rs"),
        ),
        (
            "tuya/lan.rs",
            include_str!("../../../server/src/device/tuya/lan.rs"),
        ),
        (
            "workflow/pipeline/inference.rs",
            include_str!("../../../server/src/workflow/pipeline/inference.rs"),
        ),
        (
            "workflow/pipeline/splitter.rs",
            include_str!("../../../server/src/workflow/pipeline/splitter.rs"),
        ),
    ] {
        for forbidden in ["crate::plugins::wasm", "WasmHost", "dispatch_wasm"] {
            assert!(
                !source.contains(forbidden),
                "native hot path {name} must not dispatch WASM via {forbidden}"
            );
        }
    }
}
