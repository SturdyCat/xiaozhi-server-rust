//! `plugins/memory/mod.rs` 的回归测试：配置校验、默认零成本、作用域与注入位置。
//!
//! 这些断言保护的是**开关语义**：默认关闭时不得有任何副作用，打开后的错误配置必须
//! 给出可行动的提示（而不是启动失败或静默失效）。

use super::*;
use crate::config::Config;
use crate::plugins::prompt::MemoryPosition;

fn enabled_cfg() -> MemoryConfig {
    MemoryConfig {
        enabled: true,
        db_path: ":memory:".into(),
        ..MemoryConfig::default()
    }
}

#[tokio::test]
async fn default_config_builds_noop_and_has_no_side_effects() {
    let cfg = MemoryConfig::default();
    assert!(!cfg.enabled, "默认必须关闭");
    assert!(!cfg.active());
    let m = build_memory(&cfg, &Config::default().llm).unwrap();
    assert_eq!(m.name(), "none");
    // 关闭时任何调用都是零成本的无操作
    assert!(m
        .recall(Some("xiaozhi:dev1"), "随便问")
        .await
        .block
        .is_none());
    m.record(CompletedTurn {
        scope: "xiaozhi:dev1".into(),
        session_id: "s".into(),
        user_text: "问".into(),
        assistant_text: "答".into(),
    })
    .await
    .unwrap();
    assert_eq!(m.extract_pending(5).await, ExtractOutcome::default());
    assert!(!m.maintain(MaintainOpts::periodic()).await.unwrap().ran);
    assert!(m.clear(None).await.unwrap() == 0);
    let s = m.stats();
    assert_eq!(s.engine, "none");
    assert!(!s.active);
    assert_eq!(s.memories, 0);
}

#[test]
fn backend_none_is_not_active_even_when_enabled() {
    let mut cfg = enabled_cfg();
    cfg.engine = MemoryBackend::None;
    assert!(!cfg.active(), "engine=none 时不应生效");
    assert!(cfg.validate().is_err(), "矛盾配置必须报错并说明怎么改");
    let err = format!("{:#}", cfg.validate().unwrap_err());
    assert!(err.contains("engine"), "{err}");
    assert!(err.contains("enabled"), "{err}");
}

#[test]
fn validate_rejects_empty_path_scope_and_bad_node_count() {
    let mut cfg = enabled_cfg();
    cfg.db_path = "  ".into();
    assert!(format!("{:#}", cfg.validate().unwrap_err()).contains("db_path"));

    let mut cfg = enabled_cfg();
    cfg.scope_prefix = String::new();
    assert!(format!("{:#}", cfg.validate().unwrap_err()).contains("scope_prefix"));

    let mut cfg = enabled_cfg();
    cfg.recall_max_nodes = 0;
    assert!(format!("{:#}", cfg.validate().unwrap_err()).contains("recall_max_nodes"));

    // 关闭时一律豁免（老配置不该因为新段而无法启动）
    let off = MemoryConfig {
        db_path: String::new(),
        ..MemoryConfig::default()
    };
    off.validate().unwrap();
}

#[test]
fn scope_for_is_stable_and_handles_missing_device() {
    let cfg = enabled_cfg();
    assert_eq!(cfg.scope_for("AA:BB:CC"), "xiaozhi:AA:BB:CC");
    assert_eq!(cfg.scope_for("  "), "xiaozhi:unknown");
    assert_eq!(cfg.scope_for("dev1"), cfg.scope_for("dev1"), "必须可复算");
}

#[test]
fn inject_position_parses_from_config_string() {
    let mut cfg = enabled_cfg();
    assert_eq!(
        cfg.inject_position(),
        MemoryPosition::AfterHistory,
        "默认在历史之后"
    );
    cfg.inject_position = "before_history".into();
    assert_eq!(cfg.inject_position(), MemoryPosition::BeforeHistory);
    cfg.inject_position = "垃圾值".into();
    assert_eq!(
        cfg.inject_position(),
        MemoryPosition::AfterHistory,
        "非法值回落默认"
    );
}

#[test]
fn warning_surfaces_degraded_reasons_instead_of_silence() {
    let llm = Config::default().llm;
    let cfg = enabled_cfg();
    let w = cfg.warning(&llm).expect("无 embedding 时必须给出降级原因");
    assert!(w.contains("词法"), "应说明当前是词法模式: {w}");
    // 默认 [llm] 无密钥 → 应提示抽取缺凭据
    assert!(w.contains("抽取"), "{w}");

    // 抽取路由配齐后不再提"缺凭据"，但"无 embedding"仍在
    let mut cfg = enabled_cfg();
    cfg.extractor.api_base = "https://api.example.com/v1/responses".into();
    cfg.extractor.model = "small-model".into();
    cfg.extractor.api_key = "sk-x".into();
    let w = cfg.warning(&llm).unwrap();
    assert!(!w.contains("未配置模型路由"), "{w}");
    assert!(w.contains("词法"), "{w}");
}

#[test]
fn extractor_resolve_overrides_only_non_empty_fields() {
    let mut llm = Config::default().llm;
    llm.api_base = "https://main.example.com/v1/responses".into();
    llm.api_key = "main-key".into();
    llm.model = "main-model".into();
    llm.stream = true;

    let empty = ExtractorConfig::default();
    assert!(empty.is_empty());
    let c = empty.resolve(&llm);
    assert_eq!(c.api_base, "https://main.example.com/v1/responses");
    assert_eq!(c.api_key, "main-key");
    assert_eq!(c.model, "main-model");
    assert!(!c.stream, "抽取不需要流式（要的是完整 JSON）");
    assert!((c.temperature - 0.1).abs() < f32::EPSILON);

    // 只换模型：其余字段沿用 [llm]
    let partial = ExtractorConfig {
        model: "cheap-model".into(),
        temperature: 0.0,
        ..ExtractorConfig::default()
    };
    assert!(!partial.is_empty());
    let c = partial.resolve(&llm);
    assert_eq!(c.model, "cheap-model");
    assert_eq!(c.api_key, "main-key", "未覆盖的字段必须沿用 [llm]");
    assert!(!partial.resolve_missing(&llm), "有 [llm] 密钥就不算缺凭据");
}

#[test]
fn signature_changes_with_any_field() {
    let a = enabled_cfg();
    let mut b = enabled_cfg();
    assert_eq!(a.signature(), b.signature());
    b.recall_max_nodes += 1;
    assert_ne!(
        a.signature(),
        b.signature(),
        "改参数必须改变签名（否则热切换不生效）"
    );
}
