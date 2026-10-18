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

// ============================================================
// P1-7：store 后端（多后端注册表）的配置层护栏
// ============================================================

/// PRD 验收 4：未知 engine 值 → 反序列化**直接报错**且文案含「怎么修」与合法值清单。
/// （serde derive 的英文报错不满足 NF-3，本测试守护自定义 Deserialize 的中文口径。）
///
/// ⚠️ 直接反序列化 `MemoryConfig` 时 TOML 文档**不带 `[memory]` 表头**（其字段就是文档
/// 顶层；带表头的话顶层键 "memory" 是未知字段被静默忽略，全部走默认值——测了个寂寞）。
/// 走完整 `[memory]` 表头的真实加载路径由下方 `unknown_engine_via_full_config` 覆盖。
#[test]
fn unknown_engine_reports_chinese_error_with_fix_hint() {
    let err = toml::from_str::<MemoryConfig>("enabled = true\nengine = \"store2\"\n")
        .err()
        .expect("未知 engine 必须报错");
    let msg = format!("{err}");
    assert!(msg.contains("store2"), "应回显用户写的值: {msg}");
    assert!(
        msg.contains("可选 graph / store / rag / none"),
        "必须列出合法值: {msg}"
    );
    assert!(
        msg.contains("怎么修") || msg.contains("请改"),
        "必须含修复指引: {msg}"
    );
    // ⚠️ TOML 裸字面量（engine = mistral，无引号）在 **TOML 解析层**即被拒（invalid string），
    // 到不了 serde 层——自定义 Deserialize 只需拦截"合法 TOML 字符串但取值未知"的场景。
}

/// 真实加载路径（`Config::load` 口径）：`[memory].engine` 未知值在**完整 Config** 反序列化
/// 时同样被拦截（Config::load 启动报错 / PUT /api/config 400 都经此层）。
#[test]
fn unknown_engine_via_full_config() {
    let err = toml::from_str::<Config>("[memory]\nenabled = true\nengine = \"store2\"\n")
        .err()
        .expect("完整 Config 路径未知 engine 必须报错");
    let msg = format!("{err:#}");
    assert!(msg.contains("不是已知后端"), "{msg}");
}

/// 合法取值与大小写/空白宽容度（对齐 canonical_engine 的口径）。
#[test]
fn engine_parses_graph_store_none_with_leniency() {
    let parse = |s: &str| {
        toml::from_str::<MemoryConfig>(&format!("engine = \"{s}\"\n"))
            .unwrap()
            .engine
    };
    assert_eq!(parse("graph"), MemoryBackend::Graph);
    assert_eq!(parse("store"), MemoryBackend::Store);
    assert_eq!(parse("none"), MemoryBackend::None);
    assert_eq!(parse("  Store "), MemoryBackend::Store, "trim + 大小写宽容");
    assert_eq!(parse("GRAPH"), MemoryBackend::Graph);
    // 序列化侧：写出小写（与旧部署的 TOML 形态一致）
    assert_eq!(
        toml::to_string(&MemoryConfig {
            engine: MemoryBackend::Store,
            ..MemoryConfig::default()
        })
        .unwrap()
        .contains("store"),
        true
    );
}

/// PRD 验收 5（默认回归）：不写 engine → graph；enabled=false → active() 恒 false。
#[test]
fn default_zero_cost_contract_survives_store_addition() {
    let cfg = toml::from_str::<MemoryConfig>("enabled = false\n").unwrap();
    assert_eq!(cfg.engine, MemoryBackend::Graph, "不写 engine 默认 graph");
    assert!(!cfg.active(), "enabled=false 时任何 engine 都不生效");
    // 旧键 backend 仍可读（字段级 alias 与枚举自定义 Deserialize 正交）
    let legacy = toml::from_str::<MemoryConfig>("enabled = true\nbackend = \"store\"\n").unwrap();
    assert_eq!(
        legacy.engine,
        MemoryBackend::Store,
        "旧键 backend 别名仍生效"
    );
}

/// active() 语义扩展（P1-7）：store 也算生效（写入门要放行），none 不生效。
#[test]
fn active_covers_store_but_not_none() {
    let mut cfg = enabled_cfg();
    assert!(cfg.active(), "graph + enabled 生效");
    cfg.engine = MemoryBackend::Store;
    assert!(
        cfg.active(),
        "store + enabled 必须生效（spawn_memory_write 门要放行）"
    );
    cfg.engine = MemoryBackend::None;
    assert!(!cfg.active(), "none 不生效（显式停用）");
}

/// engine_id：三个后端的稳定 id（`/api/memory/status` 与注册表对账口径）。
#[test]
fn engine_id_maps_all_backends() {
    assert_eq!(enabled_cfg().engine_id(), "graph");
    let mut s = enabled_cfg();
    s.engine = MemoryBackend::Store;
    assert_eq!(s.engine_id(), "store");
    let mut n = enabled_cfg();
    n.engine = MemoryBackend::None;
    assert_eq!(n.engine_id(), "none");
}

/// 热切换签名：engine 字段参与签名（serde 全量）→ graph⇄store 切换必触发重建。
#[test]
fn signature_covers_engine_switch() {
    let g = enabled_cfg();
    let mut s = enabled_cfg();
    s.engine = MemoryBackend::Store;
    assert_ne!(
        g.signature(),
        s.signature(),
        "engine 切换必须改变签名，否则热切换不会重建引擎"
    );
}

/// store 后端的校验：db_path / scope_prefix 检查同样必需（store 也按 scope 落库），
/// 但 graph 特有的 recall_max_nodes 范围检查对 store 不设限（无召回语义）。
#[test]
fn store_backend_validates_shared_fields() {
    let mut cfg = enabled_cfg();
    cfg.engine = MemoryBackend::Store;
    cfg.db_path = "  ".into();
    let err = format!("{:#}", cfg.validate().unwrap_err());
    assert!(err.contains("db_path"), "store 也必须有可写路径: {err}");

    let mut ok = enabled_cfg();
    ok.engine = MemoryBackend::Store;
    ok.validate().unwrap();
}

/// warning()：store 后端不给"无 embedding"类降级提示（那是 graph 的能力受限，
/// store 的"不召回"是设计定位而非缺陷——显示成降级会让用户误以为坏了）。
#[test]
fn store_backend_has_no_warning() {
    let mut cfg = enabled_cfg();
    cfg.engine = MemoryBackend::Store;
    assert!(
        cfg.warning(&Config::default().llm).is_none(),
        "store 不应有降级提示"
    );
}
