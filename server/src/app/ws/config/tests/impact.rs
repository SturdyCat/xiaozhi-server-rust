//! 保存生效档位（`POST /api/config` 的 `hot`）回归测试。
//!
//! 从 `config/tests.rs` 拆出（`AGENTS.md` §5.10 + 职责聚类）：这里只回答一个问题——
//! **「这次保存，用户该看到哪一档生效文案」**。断言锚在注册表元数据上（而非硬编码字符串），
//! 这样元数据改档位时测试会跟着变，不会变成"改了元数据但测试仍在骗人"。

use super::*;
use crate::plugins::registry::{self, HotReload};

/// 改一个**新会话生效**的字段（`[llm].model`）→ NextSession。
#[test]
fn impact_llm_model_is_next_session() {
    let mut base = full_test_config();
    base.llm.model = "gpt-4o".into();
    let after = simulate_put(
        &base,
        serde_json::json!({"llm": {"model": "gpt-4o-mini"}}),
    );
    let impact = save_impact(&base, &after);
    assert_eq!(impact.hot, HotReload::NextSession);
    assert_eq!(impact.sections, vec!["llm".to_string()]);
    assert_eq!(impact.paths, vec!["llm.model".to_string()]);
}

/// 改一个**需重启**的字段（`[server].port`）→ RestartOnly。
#[test]
fn impact_server_port_is_restart_only() {
    let base = full_test_config();
    assert_eq!(base.server.port, 8123, "fixture 端口已改则本测试需同步");
    let after = simulate_put(&base, serde_json::json!({"server": {"port": 9999}}));
    let impact = save_impact(&base, &after);
    assert_eq!(impact.hot, HotReload::RestartOnly);
    assert_eq!(impact.sections, vec!["server".to_string()]);
    assert_eq!(impact.paths, vec!["server.port".to_string()]);
}

/// **多段同改取最严格**：一次保存同时改 `[llm]`（新会话）与 `[audio]`（需重启）
/// → 对外只能承诺"需重启"（否则就是"说立即生效、其实没生效"）。
#[test]
fn impact_takes_strictest_across_sections() {
    let base = full_test_config();
    let after = simulate_put(
        &base,
        serde_json::json!({
            "llm": {"model": "gpt-4o-mini"},
            "audio": {"downlink_lead_ms": 111},
        }),
    );
    let impact = save_impact(&base, &after);
    assert_eq!(impact.hot, HotReload::RestartOnly, "多段同改必须取最严格档位");
    assert_eq!(impact.sections, vec!["audio".to_string(), "llm".to_string()]);
}

/// **无改动** → Live（"已是最新"，不该吓唬用户说需要重启）；
/// 也不能因为"整份表单回传"就误报一堆变更段。
#[test]
fn impact_no_change_is_live_and_empty() {
    let base = full_test_config();
    let after = simulate_put(&base, serde_json::json!({}));
    let impact = save_impact(&base, &after);
    assert_eq!(impact.hot, HotReload::Live);
    assert!(impact.sections.is_empty(), "无改动不应报告任何段: {:?}", impact.sections);
    assert!(impact.paths.is_empty());
}

/// 纯 **P4 旧写法迁移**不得被算成一次真实变更：
/// 旧客户端补丁写 `tts.backend` + `tts.model`，规范化后落到 `engine`/`[tts.kokoro].model`。
/// 值本身与基线一致时 → 无改动（Live）；值不同才报告，且报告的是**规范路径**。
#[test]
fn impact_normalizes_legacy_patch_before_diffing() {
    // 基线已规范：engine=kokoro，kokoro.model=<fixture 值>
    let mut base = full_test_config();
    base.tts.engine = "kokoro".into();
    let before_model = base.tts.kokoro.model.clone();

    // ① 旧写法回传**相同值** → 不算变更
    let same = simulate_put(
        &base,
        serde_json::json!({"tts": {"backend": "sherpa", "model": before_model}}),
    );
    let impact = save_impact(&base, &same);
    assert_eq!(impact.hot, HotReload::Live, "同值迁移不应算变更: {:?}", impact.paths);

    // ② 旧写法回传**不同值** → 报告规范路径（tts.kokoro.model），而不是旧路径
    let changed = simulate_put(
        &base,
        serde_json::json!({"tts": {"backend": "sherpa", "model": "/m/new.onnx"}}),
    );
    let impact = save_impact(&base, &changed);
    assert_eq!(impact.sections, vec!["tts".to_string()]);
    assert!(
        impact.paths.iter().any(|p| p == "tts.kokoro.model"),
        "应报告规范路径: {:?}",
        impact.paths
    );
    // **字段级精度**：该字段是"新会话生效"，比段级最严格档（`[tts].cache_entries` 需重启）更准
    assert_eq!(impact.hot, HotReload::NextSession);
    assert_eq!(
        registry::field_hot(&["tts", "kokoro", "model"]),
        Some(HotReload::NextSession)
    );
}

/// 变更**数组元素**（`[soul].traits` 增删一条）必须被看见：整段替换按一个变更点算，
/// 逐级回退到 `soul.traits` 的字段元数据（而不是漏报成"无改动"）。
#[test]
fn impact_detects_array_element_change() {
    let mut base = full_test_config();
    base.soul.traits = vec!["直接".into()];
    let after = simulate_put(
        &base,
        serde_json::json!({"soul": {"traits": ["直接", "温和"]}}),
    );
    let impact = save_impact(&base, &after);
    assert_eq!(impact.sections, vec!["soul".to_string()]);
    assert!(impact.paths.iter().all(|p| p.starts_with("soul")), "{:?}", impact.paths);
    assert_eq!(
        impact.hot,
        registry::field_hot(&["soul", "traits"]).expect("soul.traits 应有字段元数据")
    );
}

/// ✨**权威性护栏**：序列化后 `Config` 的**每一个顶层段**都必须在注册表里有明确档位
/// （能力字段表或 `APP_SECTION_HOT`）。
///
/// 为什么必须有：`field_hot`/`section_hot` 查不到时会保守返回 `RestartOnly`，
/// 新增配置段却忘记登记元数据时**不会报错**，只会让 UI 永远显示"需重启"——
/// 这正是本仓库最忌讳的"静默不正确"。此测试把该静默降级变成硬失败。
#[test]
fn every_config_section_has_declared_hot_semantics() {
    let v = serde_json::to_value(full_test_config()).unwrap();
    let mut missing: Vec<&str> = v
        .as_object()
        .expect("Config 序列化为对象")
        .keys()
        .map(|k| k.as_str())
        .filter(|k| registry::section_hot_known(k).is_none())
        .collect();
    missing.sort();
    assert!(
        missing.is_empty(),
        "配置段没有登记热生效语义（请补注册表字段元数据或 APP_SECTION_HOT）: {missing:?}"
    );
}

/// 字段级查询优先于段级：`[server]`（无字段元数据）走段级表；
/// `[tts.kokoro].model` 走字段元数据（粒度更细，档位可不同于段级最严格值）。
#[test]
fn field_hot_prefers_registry_field_over_section() {
    assert_eq!(registry::field_hot(&["server", "port"]), None);
    assert_eq!(registry::section_hot("server"), HotReload::RestartOnly);
    assert_eq!(
        registry::field_hot(&["tts", "kokoro", "model"]),
        Some(HotReload::NextSession)
    );
    // `[tts].cache_entries` 是需重启项 → 段级最严格档比字段级更保守（两者都合法，粒度不同）
    assert_eq!(registry::section_hot("tts"), HotReload::RestartOnly);
    assert_eq!(
        registry::field_hot(&["tts", "cache_entries"]),
        Some(HotReload::RestartOnly)
    );
}

/// `[llm].api_key`（密钥）也在注册表里登记了档位：改密钥同样要给出正确的生效承诺。
#[test]
fn secret_field_has_hot_semantics() {
    let mut base = full_test_config();
    base.llm.api_key = "old".into();
    let after = simulate_put(&base, serde_json::json!({"llm": {"api_key": "new"}}));
    let impact = save_impact(&base, &after);
    assert_eq!(impact.sections, vec!["llm".to_string()]);
    assert!(impact.paths.iter().any(|p| p == "llm.api_key"), "{:?}", impact.paths);
    assert!(impact.hot.rank() <= HotReload::RestartOnly.rank());
}
