//! `app/ws/health.rs` 的单元测试（纯函数级，无 HTTP 栈依赖）。
//!
//! 覆盖 PRD P1-6 验收 1/2/4/6 的可单测部分：
//! - 验收 1：11 条记录、字段齐全；
//! - 验收 2：memory.enabled=false → phase=disabled、last_error 空、probe null；
//! - 验收 4：必需能力（ASR）缺模型文件 → failed（无 sherpa feature 下用临时路径模拟）；
//! - 验收 6：幂等——同一配置连调两次，除 generated_ms 外逐字段一致。

use super::*;
use crate::config::Config;

/// 全部能力 11 行 + 键契约（客户端按字符串取键，缺键只会显示空白，必须钉死）。
#[test]
fn engines_health_returns_all_capabilities_with_full_keys() {
    let host = PluginHost::plan(&Config::default());
    let json = engines_health_json(&host);
    assert_eq!(json["ok"], true, "只读端点恒 ok");
    let caps = json["capabilities"].as_array().expect("capabilities 数组");
    assert_eq!(caps.len(), ALL_CAPABILITIES.len(), "应覆盖全部 {} 个能力", ALL_CAPABILITIES.len());
    for c in caps {
        for k in [
            "capability",
            "display",
            "required",
            "enabled",
            "phase",
            "deviation",
            "active_impl",
            "last_error",
            "config_section",
            "probe",
            "implementations",
        ] {
            assert!(c.get(k).is_some(), "能力 {} 缺键 {k}", c["capability"]);
        }
        let impls = c["implementations"].as_array().expect("implementations 数组");
        assert!(!impls.is_empty(), "能力 {} 应至少有一个实现", c["capability"]);
    }
    for k in ["active", "degraded", "failed", "disabled", "total"] {
        assert!(json["counts"].get(k).is_some(), "counts 缺键 {k}");
    }
}

/// 验收 2：memory.enabled=false → disabled、last_error null、probe null（显式关闭不是错误）。
#[test]
fn disabled_memory_has_no_probe_and_no_error() {
    let host = PluginHost::plan(&Config::default()); // memory 默认 enabled=false
    let json = engines_health_json(&host);
    let mem = find_cap(&json, "memory");
    assert_eq!(mem["phase"], "disabled");
    assert!(mem["last_error"].is_null());
    assert!(mem["probe"].is_null(), "未启用的能力不跑探测");
    assert!(!mem["deviation"].as_bool().unwrap(), "disabled 不是偏离");
}

/// 验收 4：必需能力（ASR）缺模型文件 → phase=failed（probe Fail + required → Failed）。
#[test]
fn required_capability_missing_model_reports_failed() {
    let mut cfg = Config::default();
    cfg.asr.model = "/tmp/__xiaozhi_health_never_exists__/model.onnx".into();
    let host = PluginHost::plan(&cfg);
    let json = engines_health_json(&host);
    let asr = find_cap(&json, "asr");
    assert_eq!(asr["phase"], "failed", "必需能力缺模型应合并为 failed");
    let probe = asr["probe"].as_object().expect("asr 应有探测结论");
    assert_eq!(probe["state"], "fail");
    // 原因必须非空且含「怎么修」口径：sherpa 构建下是探测文案（含"不存在"），
    // 非 sherpa 构建下是 host 的 feature 缺失提示（含"sherpa"）——两者都是可行动的。
    let err = asr["last_error"].as_str().expect("failed 必须带原因");
    assert!(
        err.contains("不存在") || err.contains("sherpa") || err.contains("请"),
        "原因应可行动: {err}"
    );
}

/// 可选能力（memory）目录不可写/不存在 → degraded（坏掉只降级，不 failed）。
/// ⚠️ 非 sherpa 构建下 memory 无 feature 门槛（plan 不 Fail），探测文案能透出；
/// sherpa 构建下同样如此（memory.validate 不依赖 feature）。
#[test]
fn optional_capability_probe_fail_degrades_not_fails() {
    let mut cfg = Config::default();
    cfg.memory.enabled = true;
    cfg.memory.db_path = "/tmp/__xiaozhi_health_never_exists_dir__/memory.db".into();
    let host = PluginHost::plan(&cfg);
    let json = engines_health_json(&host);
    let mem = find_cap(&json, "memory");
    assert_eq!(mem["phase"], "degraded", "可选能力探测失败应降级");
    let err = mem["last_error"].as_str().expect("降级必须带原因");
    assert!(err.contains("db_path") || err.contains("目录"), "应点名字段: {err}");
}

/// host 侧已有更坏状态时保留 host 的原因（启动期根因优先，探测文案不覆盖）。
#[test]
fn host_failure_reason_wins_over_probe_when_worse() {
    // 非 sherpa 构建下 asr.plan 直接 Failed（feature 缺失）；把模型路径同时指向不存在的
    // 文件，验证合并后 phase=failed 且原因是 host 的（含 sherpa 字样）而非探测文案。
    let mut cfg = Config::default();
    cfg.asr.model = "/tmp/__xiaozhi_health_never_exists__/m.onnx".into();
    let host = PluginHost::plan(&cfg);
    let json = engines_health_json(&host);
    let asr = find_cap(&json, "asr");
    assert_eq!(asr["phase"], "failed");
    let s = host.capability_summary(Capability::Asr);
    if s.phase == Phase::Failed {
        let host_err = s.last_error.unwrap_or_default();
        let got = asr["last_error"].as_str().unwrap_or_default();
        assert!(
            got == host_err || got.contains("不存在"),
            "host 更坏时保留 host 原因（或同为探测文案）: {got}"
        );
    }
}

/// config_section 映射：fullchain→aiui、firmware→ota、voices→null、其余同 id。
#[test]
fn config_section_mapping_matches_admin_nav() {
    let host = PluginHost::plan(&Config::default());
    let json = engines_health_json(&host);
    assert_eq!(find_cap(&json, "fullchain")["config_section"], "aiui");
    assert_eq!(find_cap(&json, "firmware")["config_section"], "ota");
    assert!(find_cap(&json, "voices")["config_section"].is_null());
    assert_eq!(find_cap(&json, "memory")["config_section"], "memory");
    assert_eq!(find_cap(&json, "wakeup")["config_section"], "wakeup");
}

/// 同源保证：/api/plugins 与 /api/health/engines 的能力级 phase/enabled/active_impl
/// 完全一致（probe 未 Fail 时两边同值——分叉即测试红）。
#[test]
fn same_source_as_plugins_endpoint() {
    let mut cfg = Config::default();
    cfg.llm.api_key = "sk-t".into();
    let host = PluginHost::plan(&cfg);
    let plugins = host.capabilities_json();
    let health = engines_health_json(&host);
    for h in health["capabilities"].as_array().unwrap() {
        let id = h["capability"].as_str().unwrap();
        let p = plugins["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .unwrap_or_else(|| panic!("健康端点能力 {id} 不在 /api/plugins"));
        // 探测未让 phase 变坏时两边必须一致（默认配置下模型路径存在性两边都看到了：
        // /api/plugins 不做探测，因此仅在探测 Ok 时两边相同）。
        let probe_ok = h["probe"].is_null()
            || h["probe"]["state"] == "ok";
        if probe_ok {
            assert_eq!(h["phase"], p["phase"], "能力 {id} 两端点 phase 分叉");
        }
        assert_eq!(h["enabled"], p["enabled"], "能力 {id} enabled 分叉");
        assert_eq!(h["active_impl"], p["active_impl"], "能力 {id} active_impl 分叉");
        // 实现明细同构：逐条 id 一致
        let hi: Vec<&str> = h["implementations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect();
        let pi: Vec<&str> = p["implementations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap())
            .collect();
        assert_eq!(hi, pi, "能力 {id} 实现明细分叉");
    }
}

/// 验收 6（幂等）：同一配置连调两次，除 generated_ms 外逐字段一致。
#[test]
fn engines_health_is_idempotent() {
    let mut cfg = Config::default();
    cfg.memory.enabled = true;
    cfg.memory.db_path = ":memory:".into();
    let host = PluginHost::plan(&cfg);
    let a = engines_health_json(&host);
    let b = engines_health_json(&host);
    let mut ac = a.clone();
    let mut bc = b.clone();
    ac["generated_ms"].take();
    bc["generated_ms"].take();
    assert_eq!(
        serde_json::to_string(&ac).unwrap(),
        serde_json::to_string(&bc).unwrap(),
        "两次调用除 generated_ms 外应逐字段一致"
    );
}

/// 能力聚合取最坏：health 端点也必须看到 host 标记的降级（mark 后 phase 透出）。
#[test]
fn marked_degradation_is_visible_in_health() {
    let host = PluginHost::plan(&Config::default());
    host.mark(
        "voices.catalog",
        Phase::Degraded,
        Some("发音人目录探测不可用（缺凭据），已跳过".into()),
    );
    let json = engines_health_json(&host);
    let voices = find_cap(&json, "voices");
    assert_eq!(voices["phase"], "degraded");
    assert!(voices["last_error"].as_str().unwrap().contains("凭据"));
    // 默认配置（非 sherpa）下 asr/tts.kokoro 可能已 failed，只断言 degraded ≥ 1
    assert!(json["counts"]["degraded"].as_u64().unwrap() >= 1);
}

fn find_cap<'a>(json: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    json["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["capability"] == id)
        .unwrap_or_else(|| panic!("缺少能力 {id}"))
}
