//! `/api/config` 的**密钥语义**回归测试：打码、presence、「空值 = 保留原值」、指纹无关性。
//!
//! 从 `config/tests.rs` 拆出（`AGENTS.md` §5.10 文件规模约定 + 职责聚类）：
//! 这里全部围绕"密钥/指纹"，与部分更新语义（留在父模块）分开看更清楚。
//! ⚠️ 使用父模块的 `full_test_config()` / `simulate_put()`：子模块可访问祖先的私有项。

use super::*;

/// GET 打码：非空密钥被**摘除**只留 presence 标记，空密钥标记为 `false`，非密钥字段不受影响。
#[test]
fn get_redacts_secrets_and_reports_presence() {
    let mut cfg = full_test_config();
    cfg.llm.api_key = "sk-live-123".into();
    cfg.tts.xfyun.api_key = "xf-k".into();
    cfg.tts.xfyun.api_secret = String::new(); // 未配置
    cfg.aiui.api_secret = "aiui-sec".into();

    let mut v = serde_json::to_value(&cfg).unwrap();
    redact_secrets(&mut v);
    let text = v.to_string();

    // ① 所有非空密钥明文都不得作为**JSON 值**出现在响应里
    //   （按带引号形式匹配：fixture 里的短密钥如 "ak" 是 "speaker" 的子串，裸 contains 会误报）
    let secrets = [
        &cfg.llm.api_key,
        &cfg.tts.xfyun.api_key,
        &cfg.tts.xfyun.api_secret,
        &cfg.aiui.api_key,
        &cfg.aiui.api_secret,
    ];
    let non_empty: Vec<&String> = secrets.iter().filter(|s| !s.is_empty()).copied().collect();
    assert!(non_empty.len() >= 2, "fixture 应含非空密钥");
    for s in non_empty {
        assert!(
            !text.contains(&format!("\"{s}\"")),
            "密钥明文不得出现在 GET 响应: {s}"
        );
    }
    // ② presence 标记 + 明文确实被摘除（五个路径逐个断言）
    for (parent, leaf) in [
        (&v["llm"], "api_key"),
        (&v["tts"]["xfyun"], "api_key"),
        (&v["tts"]["xfyun"], "api_secret"),
        (&v["aiui"], "api_key"),
        (&v["aiui"], "api_secret"),
    ] {
        assert!(
            parent.get(leaf).is_none(),
            "{leaf} 明文不应出现在响应中（应为摘除，而不是打星号）"
        );
        assert!(
            parent.get(format!("has_{leaf}").as_str()).is_some(),
            "{leaf} 必须给出 presence 标记"
        );
    }
    assert_eq!(v["llm"]["has_api_key"], true);
    assert_eq!(v["tts"]["xfyun"]["has_api_key"], true);
    assert_eq!(
        v["tts"]["xfyun"]["has_api_secret"], false,
        "空密钥 = 未配置"
    );
    assert_eq!(v["aiui"]["has_api_secret"], true);
    // ③ 非密钥字段原样保留
    assert_eq!(v["llm"]["model"], cfg.llm.model);
    assert_eq!(v["tts"]["xfyun"]["app_id"], cfg.tts.xfyun.app_id);
    assert_eq!(v["tts"]["xfyun"]["voice"], cfg.tts.xfyun.voice);
    // ④ 打码后的响应仍是合法 Config（presence 属未知键被忽略，密钥回落到默认空值）
    let back: Config = serde_json::from_value(v).unwrap();
    assert_eq!(back.llm.api_key, "");
    assert_eq!(back.tts.xfyun.app_id, cfg.tts.xfyun.app_id);
}

/// POST 密钥语义：**空串/缺失 = 保留原值**（旧客户端回传空串不再清空密钥）、
/// 显式 `null` = 清除、非空 = 覆盖；普通字段不受此逻辑影响。
#[test]
fn blank_secret_keeps_existing_and_null_clears() {
    let mut current = full_test_config();
    current.llm.api_key = "sk-existing".into();
    current.tts.xfyun.api_secret = "sec-existing".into();

    // 旧客户端把（已打码的）GET 结果原样回传：空串 + presence 标记 + revision
    let echo = serde_json::json!({
        "revision": "deadbeef",
        "llm": {"api_key": "", "has_api_key": true, "model": "gpt-4o-mini"},
        "tts": {"xfyun": {"app_id": "app2", "api_secret": ""}},
    });
    let out = simulate_put(&current, echo);
    assert_eq!(out.llm.api_key, "sk-existing", "空串不得覆盖已存密钥");
    assert_eq!(out.tts.xfyun.api_secret, "sec-existing");
    assert_eq!(out.llm.model, "gpt-4o-mini", "普通字段仍然生效");
    assert_eq!(out.tts.xfyun.app_id, "app2");

    // 显式 null = 清除（回落默认值）
    let out = simulate_put(&current, serde_json::json!({"llm": {"api_key": null}}));
    assert_eq!(out.llm.api_key, "");

    // 非空 = 覆盖
    let out = simulate_put(&current, serde_json::json!({"llm": {"api_key": "sk-new"}}));
    assert_eq!(out.llm.api_key, "sk-new");
}

/// 指纹基于**打码后**的内容：不随密钥值变化（避免指纹成为密钥比对依据），
/// 但随密钥"有无"（presence）变化。
#[test]
fn revision_ignores_secret_values_but_tracks_presence() {
    let mut a = full_test_config();
    a.llm.api_key = "sk-aaa".into();
    let mut b = a.clone();
    b.llm.api_key = "sk-bbb".into();
    assert_eq!(
        config_revision(&a),
        config_revision(&b),
        "改密钥值不应改变指纹"
    );
    let mut c = b.clone();
    c.llm.api_key = String::new();
    assert_ne!(
        config_revision(&a),
        config_revision(&c),
        "密钥有无（presence）应改变指纹"
    );
}

/// presence 标记与 `revision` 同为**响应包装字段**：客户端原样回传后不得写进配置。
#[test]
fn presence_and_revision_keys_never_persist() {
    let current = full_test_config();
    let echo = serde_json::json!({
        "revision": "ff",
        "expected_revision": "ff",
        "llm": {"has_api_key": true},
        "tts": {"xfyun": {"has_api_secret": false}},
    });
    let out = simulate_put(&current, echo);
    let text = toml::to_string_pretty(&out).unwrap();
    assert!(!text.contains("has_api_key"), "presence 标记不得写入配置");
    assert!(!text.contains("has_api_secret"));
    assert!(!text.contains("revision"));
}

/// `[echomuse]`：`token` 已入 [`SECRET_PATHS`]（GET 打码 + 空串保留原值），
/// `enabled` 是普通布尔字段、经管理台补丁**透传生效**（sanitize 不丢段）。
#[cfg(feature = "native-plugins")]
#[test]
fn echomuse_token_is_secret_and_enabled_passes_through() {
    let mut current = full_test_config();
    current.echomuse.enabled = false;
    current.echomuse.token = "em-pair-secret".into();

    // GET 打码：token 明文摘除，只回 has_token presence
    let mut v = serde_json::to_value(&current).unwrap();
    redact_secrets(&mut v);
    assert!(
        v["echomuse"].get("token").is_none(),
        "token 明文不应出现在 GET 响应中"
    );
    assert_eq!(v["echomuse"]["has_token"], true);

    // 旧客户端把打码后的 GET 结果原样回传：空串 + presence 不清密钥；enabled 透传生效
    let echo = serde_json::json!({
        "echomuse": {"enabled": true, "token": "", "has_token": true},
    });
    let out = simulate_put(&current, echo);
    assert!(out.echomuse.enabled, "echomuse.enabled 应透传生效");
    assert_eq!(out.echomuse.token, "em-pair-secret", "空串 token 保留原值");
}

/// `[tuya]`：`access_secret` 已入 [`SECRET_PATHS`]（GET 打码 + 空串保留原值）；
/// `access_id` 是 client_id 标识、非密钥，明文保留。
#[cfg(feature = "native-plugins")]
#[test]
fn tuya_access_secret_is_redacted_but_access_id_stays() {
    let mut current = full_test_config();
    current.tuya.access_id = "cloud-client-id".into();
    current.tuya.access_secret = "cloud-secret".into();

    // GET 打码：secret 明文摘除只回 presence；access_id 原样保留
    let mut v = serde_json::to_value(&current).unwrap();
    redact_secrets(&mut v);
    assert!(
        v["tuya"].get("access_secret").is_none(),
        "access_secret 明文不应出现在 GET 响应中"
    );
    assert_eq!(v["tuya"]["has_access_secret"], true);
    assert_eq!(
        v["tuya"]["access_id"], "cloud-client-id",
        "access_id 是标识不是密钥，保留明文"
    );

    // 旧客户端回传打码后的 GET 结果（secret 空串 + presence）不清密钥；非空覆盖
    let echo = serde_json::json!({
        "tuya": {"access_id": "cloud-client-id-2", "access_secret": "", "has_access_secret": true},
    });
    let out = simulate_put(&current, echo);
    assert_eq!(out.tuya.access_id, "cloud-client-id-2");
    assert_eq!(
        out.tuya.access_secret, "cloud-secret",
        "空串 secret 保留原值"
    );
    let out = simulate_put(
        &current,
        serde_json::json!({"tuya": {"access_secret": "new-secret"}}),
    );
    assert_eq!(out.tuya.access_secret, "new-secret");
}

/// 数组型敏感字段（`[[llm.fallback.providers]]`）：GET 逐元素摘除 `api_key` 留 presence；
/// POST 整数组替换时，元素缺 `api_key` 从基线**同下标**回填（整表单回传不清密钥），
/// 显式 `null` 仍可清除。
#[test]
fn fallback_provider_keys_are_redacted_per_element() {
    let mut current = full_test_config();
    current.llm.fallback.providers = vec![
        crate::plugins::llm::LlmFallbackProvider {
            api_base: "https://backup-a.example.com/v1/responses".into(),
            api_key: "sk-backup-a".into(),
            model: "m-a".into(),
        },
        crate::plugins::llm::LlmFallbackProvider {
            api_base: "https://backup-b.example.com/v1/responses".into(),
            api_key: "sk-backup-b".into(),
            model: "m-b".into(),
        },
    ];

    // GET 打码：两个元素的密钥都摘除，presence 分别为 true
    let mut v = serde_json::to_value(&current).unwrap();
    redact_secrets(&mut v);
    let text = v.to_string();
    assert!(!text.contains("\"sk-backup-a\"") && !text.contains("\"sk-backup-b\""));
    let providers = v["llm"]["fallback"]["providers"].as_array().unwrap();
    assert_eq!(providers[0]["has_api_key"], true);
    assert_eq!(providers[1]["has_api_key"], true);
    assert!(providers[0].get("api_key").is_none());

    // POST：只改第 0 条的 model（api_key 被打码摘除，元素里没有该键）→ 密钥按同下标保留
    let patch = serde_json::json!({
        "llm": {"fallback": {"providers": [
            {"api_base": "https://backup-a.example.com/v1/responses", "model": "m-a-new"},
            {"api_base": "https://backup-b.example.com/v1/responses", "model": "m-b"},
        ]}},
    });
    let out = simulate_put(&current, patch);
    assert_eq!(out.llm.fallback.providers[0].model, "m-a-new");
    assert_eq!(
        out.llm.fallback.providers[0].api_key, "sk-backup-a",
        "缺失的元素密钥应从基线回填，不得清空"
    );
    assert_eq!(out.llm.fallback.providers[1].api_key, "sk-backup-b");

    // 显式 null = 清除该元素的密钥
    let patch = serde_json::json!({
        "llm": {"fallback": {"providers": [
            {"api_base": "https://backup-a.example.com/v1/responses", "model": "m-a", "api_key": null},
            {"api_base": "https://backup-b.example.com/v1/responses", "model": "m-b", "api_key": "sk-new-b"},
        ]}},
    });
    let out = simulate_put(&current, patch);
    assert_eq!(out.llm.fallback.providers[0].api_key, "");
    assert_eq!(out.llm.fallback.providers[1].api_key, "sk-new-b");
}
