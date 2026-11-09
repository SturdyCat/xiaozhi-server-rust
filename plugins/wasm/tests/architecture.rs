//! P8 architecture guards: WASM bridges stay above native data-plane modules.

#[test]
fn legacy_bridge_modules_are_physically_gone() {
    // 阶段二收口：遗留策略桥与事件瀑布必须不物理存在（宿主 crate 不持有任何
    // 业务扩展点名；prepare 派发归位到各能力/设备模块）。路径存在性在运行期
    // 断言（相对 CARGO_MANIFEST_DIR，即 server/）。
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for rel in [
        "../plugins/wasm/bridge.rs",
        "../plugins/wasm/bridge/policies.rs",
        "../plugins/wasm/bridge/nlu.rs",
        "../plugins/wasm/waterfall.rs",
    ] {
        assert!(
            !root.join(rel).exists(),
            "{rel} 已在阶段二删除，不得复活（新 seam 归位到能力/设备模块）"
        );
    }
}

#[test]
fn device_policy_seams_keep_native_route_contract_explicit() {
    // 归位后的设备策略 seam 仍固定「WASM 只做有界控制面决策、route=native」契约
    let tuya = include_str!("../../../server/src/device/tuya/policy.rs");
    let echomuse = include_str!("../../../server/src/device/echomuse/policy.rs");
    let esp_home = include_str!("../../../server/src/device/esp_home/policy.rs");
    assert!(tuya.contains("route != \"native\""));
    assert!(echomuse.contains("route != \"native\""));
    assert!(esp_home.contains("route != \"native\""));
}

#[test]
fn native_data_plane_ownership_markers_remain_present_before_cleanup() {
    // legacy lan effect 已删除（阶段三收口）：tuya 数据面所有权回到 worker/lan 原生路径
    let tuya_worker = include_str!("../../../server/src/device/tuya/worker.rs");
    let tuya_lan = include_str!("../../../server/src/device/tuya/lan.rs");
    let echomuse = include_str!("../../../server/src/device/echomuse/runtime.rs");
    let memory = include_str!("../../memory/mod.rs");
    assert!(!tuya_worker.contains("plugins::wasm"));
    assert!(!tuya_lan.contains("plugins::wasm"));
    assert!(echomuse.contains("hub_remove_session") || echomuse.contains("session"));
    assert!(memory.contains("MemoryProvider") || memory.contains("GraphMemory"));
}

#[test]
fn runtime_keeps_artifact_integrity_gate_before_wasm_load() {
    let boot = include_str!("../../../server/src/runtime/wasm_boot.rs");
    assert!(boot.contains("artifact_matches"));
    assert!(boot.contains("ArtifactDir::digest"));
    assert!(boot.contains("continue;"));
}

#[test]
fn weather_skill_entry_stays_plugin_dispatched_without_native_networking() {
    // geo/天气入口归属裁决（阶段二收口）：`server/src/skills/weather_ops.rs` 是通用
    // 技能步执行器——只取全局天气服务并透传 forecast，自身不联网（skills 目录铁律）。
    // 网络要么在 native NMC 客户端（native-plugins feature），要么是 WASM 插件的
    // http 效果（pure-base 经 skill.weather 扩展点派发，见 plugins/weather/dispatch.rs）；
    // `plugins/geo` 是底座通用「服务器事实/观测」能力（管理台展示 + 天气默认定位预填），
    // 不是设备业务，归属底座。
    let entry = include_str!("../../../server/src/skills/weather_ops.rs");
    assert!(
        !entry.contains("reqwest::"),
        "天气技能步入口不得自带 HTTP 客户端"
    );
    assert!(!entry.contains("TcpStream"), "天气技能步入口不得自带 TCP");
    assert!(
        entry.contains("plugins::weather::global()"),
        "天气技能步必须经 plugins::weather 能力抽象（native NMC 或 WASM 插件后端）"
    );
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
    // esp_home 的选择谓词与描述符都门控在 `esp_home` feature 下，只在该组合断言。
    #[cfg(feature = "esp_home")]
    assert!(
        selection.contains("cfg.esp_home.engine_id() == \"wasm-policy\""),
        "missing explicit selection predicate: cfg.esp_home.engine_id()"
    );
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
