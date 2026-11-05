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
    let tuya = include_str!("../../../device/tuya/wasm.rs");
    let echomuse = include_str!("../../../device/echomuse/runtime.rs");
    let memory = include_str!("../../memory/mod.rs");
    assert!(tuya.contains("drain_wasm_commands"));
    assert!(tuya.contains("set_dp"));
    assert!(echomuse.contains("hub_remove_session") || echomuse.contains("session"));
    assert!(memory.contains("MemoryProvider") || memory.contains("GraphMemory"));
}
