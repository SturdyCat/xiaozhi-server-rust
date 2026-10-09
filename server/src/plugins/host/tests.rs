//! `server/src/plugins/host.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;

/// 规划阶段：默认配置下必需能力应全部 Active（非 sherpa 构建下 ASR/TTS 为 Failed）。
#[test]
fn plan_reports_required_capabilities() {
    let host = PluginHost::plan(&Config::default());
    let snap = host.snapshot();
    assert_eq!(snap.len(), REGISTRY.len(), "状态条数应与注册表一致");

    let vad = snap.iter().find(|s| s.id == "vad.silero").unwrap();
    assert!(vad.enabled);
    assert_eq!(vad.phase, Phase::Active, "默认 VAD 模型路径非空 → 应可构建");

    // 默认配置 api_key 为空 → **Degraded 且带原因**（服务能跑，但每次对话都会 401）。
    // 这正是"enabled/phase 分离 + 软性问题上报"的实例：不是失败，但绝不能显示为正常。
    let llm = snap.iter().find(|s| s.id == "llm.openai").unwrap();
    assert_eq!(llm.phase, Phase::Degraded);
    assert!(llm.last_error.as_deref().unwrap_or("").contains("api_key"));

    // 填上密钥后应为 Active
    let mut with_key = Config::default();
    with_key.llm.api_key = "sk-test".into();
    let host2 = PluginHost::plan(&with_key);
    assert_eq!(
        host2
            .snapshot()
            .into_iter()
            .find(|s| s.id == "llm.openai")
            .unwrap()
            .phase,
        Phase::Active
    );

    let xfyun = snap.iter().find(|s| s.id == "tts.xfyun").unwrap();
    assert!(!xfyun.enabled);
    assert_eq!(xfyun.phase, Phase::Disabled, "未选中 ≠ 失败");

    let aiui = snap.iter().find(|s| s.id == "fullchain.aiui").unwrap();
    assert_eq!(aiui.phase, Phase::Disabled, "aiui 默认关闭");
}

/// 全链路模式启用但三要素缺失 → **快速失败**（保持 `Engines::new` 既有行为：
/// 配置不全时每个会话都会失败，远不如启动就报清楚）。
#[test]
fn aiui_missing_creds_fails_fast() {
    let mut cfg = Config::default();
    cfg.aiui.enabled = true; // 三要素为空
    let host = PluginHost::plan(&cfg);
    let aiui = host
        .snapshot()
        .into_iter()
        .find(|s| s.id == "fullchain.aiui")
        .unwrap();
    assert_eq!(aiui.phase, Phase::Failed);
    assert!(aiui.enabled, "用户确实启用了它（enabled 与 phase 分离）");
    let err = aiui.last_error.expect("失败必须带原因");
    assert!(
        err.contains("appid") && err.contains("enabled"),
        "原因应含怎么修: {err}"
    );
    let boot_err = PluginHost::boot(&cfg)
        .err()
        .expect("启用全链路但缺三要素应启动失败")
        .to_string();
    assert!(boot_err.contains("fullchain.aiui"), "{boot_err}");
}

/// 可选能力的降级路径：状态可被标记为 Degraded 并带原因上报
/// （当前 shipped 描述符里没有会降级的例子，故直接走 `mark` 验证机制本身）。
#[test]
fn degraded_phase_is_reported_with_reason() {
    let host = PluginHost::plan(&Config::default());
    let before = host.capabilities_json()["counts"].clone();
    host.mark(
        "voices.catalog",
        Phase::Degraded,
        Some("发音人目录探测不可用（缺凭据），已跳过".into()),
    );
    let json = host.capabilities_json();
    let voices = json["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "voices")
        .unwrap();
    assert_eq!(voices["phase"], "degraded");
    assert_eq!(
        voices["deviation"], true,
        "降级属于偏离正常，UI 应打标并给原因"
    );
    assert!(voices["last_error"].as_str().unwrap().contains("凭据"));
    // 降级**不算失败**：与标记前相比 failed 不变、degraded 恰好 +1
    assert_eq!(json["counts"]["failed"], before["failed"]);
    assert_eq!(
        json["counts"]["degraded"].as_u64().unwrap(),
        before["degraded"].as_u64().unwrap() + 1
    );
}

/// 必需能力校验失败一次性汇总报出，且不在 plan 阶段抛错。
#[test]
fn boot_reports_all_required_problems_at_once() {
    let mut cfg = Config::default();
    cfg.vad.model = String::new();
    cfg.asr.model = String::new();
    let err = PluginHost::boot(&cfg)
        .err()
        .expect("必需能力缺失应启动失败")
        .to_string();
    if cfg!(feature = "sherpa") {
        assert!(err.contains("asr.sensevoice"), "应点名 asr: {err}");
        assert!(err.contains("vad.silero"), "应点名 vad: {err}");
        assert!(err.contains("2 项"), "应汇总数量: {err}");
    } else {
        // 非 sherpa 构建：ASR/TTS 先因 feature 报错（同样是「怎么修」文案）
        assert!(err.contains("sherpa"), "{err}");
    }
}

/// 能力级聚合：disabled 实现不掩盖已激活实现。
#[test]
fn capability_aggregation_prefers_worst_phase() {
    let mut cfg = Config::default();
    // 切到 xfyun 需要凭据：缺凭据是**致命**错误（保持既有启动失败语义），故此处填齐
    cfg.tts.engine = "xfyun".into();
    cfg.tts.xfyun.app_id = "app".into();
    cfg.tts.xfyun.api_key = "key".into();
    cfg.tts.xfyun.api_secret = "secret".into();
    let host = PluginHost::plan(&cfg);
    let json = host.capabilities_json();
    let tts = json["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "tts")
        .unwrap();
    assert_eq!(tts["phase"], "active");
    assert_eq!(tts["active_impl"], "tts.xfyun");
    assert_eq!(tts["enabled"], true);
    assert_eq!(tts["deviation"], false);
    // 计数：xfyun 选中后 kokoro 应为 disabled；ASR/TTS 本地实现在无 sherpa 构建下为 failed
    if cfg!(feature = "sherpa") {
        assert_eq!(json["counts"]["failed"], 0);
    }
    assert!(json["counts"]["active"].as_u64().unwrap() >= 3);
    assert_eq!(json["counts"]["total"], REGISTRY.len());
}

/// 状态 JSON **绝不含**签名（可能含密钥）。
#[test]
fn status_json_never_leaks_signature() {
    let mut cfg = Config::default();
    cfg.llm.api_key = "sk-super-secret".into();
    cfg.tts.engine = "xfyun".into();
    cfg.tts.xfyun.api_key = "xfyun-secret".into();
    let host = PluginHost::plan(&cfg);
    let dump = serde_json::to_string(&host.capabilities_json()).unwrap();
    assert!(!dump.contains("sk-super-secret"), "状态接口泄漏了 LLM 密钥");
    assert!(!dump.contains("xfyun-secret"), "状态接口泄漏了 TTS 密钥");
    // 也不应泄漏任何 signature 字段名
    assert!(!dump.contains("signature"));
}

// ---------- P4：`engine` 键写错必须显式报错（不静默回退默认）----------

/// `/api/plugins` 侧：`engine` 写错 → 该能力所有实现均标 `Failed` 且带"可选值"原因。
/// （若只靠 `is_selected` 判否，UI 上会显示成"禁用"，用户会以为是自己关掉的。）
#[test]
fn unknown_engine_marks_capability_failed_with_hint() {
    let mut cfg = Config::default();
    cfg.tts.engine = "azure".into();
    let host = PluginHost::plan(&cfg);
    let snap = host.snapshot();
    for s in snap.iter().filter(|s| s.capability == Capability::Tts) {
        assert_eq!(s.phase, Phase::Failed, "{} 应报 Failed", s.id);
        let err = s.last_error.as_deref().unwrap_or("");
        assert!(err.contains("azure"), "{err}");
        assert!(err.contains("kokoro"), "必须列出可选值: {err}");
        assert!(!s.enabled, "没有实现被选中");
    }
    // 其他能力不受影响
    let vad = snap.iter().find(|s| s.id == "vad.silero").unwrap();
    assert_eq!(vad.phase, Phase::Active);
}

/// 启动侧：必需能力的 `engine` 写错 → `boot` 快速失败，且文案里能看出"可选什么"。
#[test]
fn boot_rejects_unknown_engine_with_actionable_message() {
    let mut cfg = Config::default();
    cfg.asr.engine = "whisper".into();
    let err = format!("{:#}", PluginHost::boot(&cfg).err().expect("应失败"));
    assert!(err.contains("whisper"), "{err}");
    assert!(err.contains("sensevoice"), "报错必须给出可选值: {err}");
}

/// `[memory].engine = "none"` 是**合法的显式停用**，不得被当成"未知实现"报错。
#[test]
fn memory_engine_none_is_not_an_unknown_engine() {
    let mut cfg = Config::default();
    cfg.memory.enabled = true;
    cfg.memory.engine = crate::plugins::memory::MemoryBackend::None;
    let host = PluginHost::plan(&cfg);
    let mem = host
        .snapshot()
        .into_iter()
        .find(|s| s.id == "memory.graph")
        .unwrap();
    assert!(!mem.enabled, "engine=none 不选中实现");
    assert_eq!(mem.phase, Phase::Disabled, "显式停用 ≠ 失败");
    assert!(crate::plugins::registry::unknown_engine_of(&cfg, Capability::Memory).is_none());
}

/// ✨**管理页契约**：总览页（`GET /api/plugins`）解析的每个键都必须存在。
///
/// 为什么值得一条测试：客户端是**字符串取键**的（`c.optString("display")` 等），
/// 服务端改名/漏字段**不会**编译报错，只会让总览页显示成一片空白——
/// 而"一片空白"恰恰是最难归因的失败形态（用户只会说"这页没用"）。
/// 这里把契约钉死：缺键即测试失败。
#[test]
fn capabilities_json_carries_every_key_the_overview_page_reads() {
    let host = PluginHost::plan(&Config::default());
    let json = host.capabilities_json();
    let caps = json["capabilities"].as_array().expect("capabilities 数组");
    assert!(!caps.is_empty());
    for c in caps {
        for k in [
            "id",
            "display",
            "required",
            "enabled",
            "phase",
            "deviation",
            "active_impl",
            "hot",
            "last_error",
            "implementations",
        ] {
            assert!(c.get(k).is_some(), "能力 {} 缺键 {k}", c["id"]);
        }
        // `last_error` 允许为 null（正常状态），但键必须在（客户端用 optString 取，缺键=空串）
        assert!(c["last_error"].is_null() || c["last_error"].is_string());
        let impls = c["implementations"].as_array().expect("implementations 数组");
        assert!(!impls.is_empty(), "能力 {} 应至少有一个实现", c["id"]);
        for im in impls {
            for k in ["id", "display", "local", "hot", "hot_hint", "enabled", "phase"] {
                assert!(im.get(k).is_some(), "实现 {} 缺键 {k}", im["id"]);
            }
            // `active_impl` 若指向某实现，客户端要靠它的 display 显示"当前用的是什么"
            assert!(
                im["hot_hint"].as_str().is_some_and(|h| !h.is_empty()),
                "hot_hint 不能为空（总览行会显示它）: {im}"
            );
        }
        // 选中态与 phase 必须自洽：选中了就不该是 disabled
        if !c["active_impl"].is_null() {
            assert!(c["enabled"].as_bool().unwrap_or(false), "{}", c["id"]);
            assert_ne!(c["phase"], "disabled", "{}", c["id"]);
        }
    }
    // 计数摘要（总览页顶部那行）依赖这四项
    for k in ["active", "degraded", "failed", "disabled", "total"] {
        assert!(json["counts"].get(k).is_some(), "counts 缺键 {k}");
    }
}
