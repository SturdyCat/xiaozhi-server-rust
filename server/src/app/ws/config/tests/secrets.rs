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
