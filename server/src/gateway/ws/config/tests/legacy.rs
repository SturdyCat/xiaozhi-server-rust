//! 既有回归测试（合并/持久化/revision）：从父模块 `tests.rs` 拆出
//! （AGENTS.md §5.10 文件规模约定：行为不变的搬移）。使用父模块 fixture。

use super::*;

// ---------- 既有回归 ----------

/// JSON 形状兼容：macApp 提交的 JSON 经「合并 → 反序列化」后类型都对得上。
#[test]
fn client_json_shape_parses_through_merge() {
    let cfg = simulate_put(
        &Config::default(),
        serde_json::json!({
            "server": {"port": 8000, "expected_token": "tk", "worker_threads": 2, "admin_dir": "/app/web"},
            "tts": {"backend": "xfyun", "xfyun": {"app_id": "app", "voice": "xiaoyan"}},
            "llm": {"api_base": "https://api.example.com/v1/responses", "model": "gpt-4o", "stream": true}
        }),
    );
    assert_eq!(cfg.server.port, 8000);
    // 形状仍是 P4 之前的（`backend` + 扁平字段）：补丁重写后同样生效
    assert_eq!(cfg.tts.engine, "xfyun");
    assert_eq!(cfg.tts.xfyun.voice, "xiaoyan");
    assert!(cfg.llm.stream);
    // 未提及的字段来自基线（默认配置）
    assert_eq!(cfg.tts.cache_entries, 256);
}

/// 持久化判定：目录挂载下的文件、文件级 bind mount 均算持久；容器层文件不算。
#[test]
fn persistence_detection() {
    let mi = concat!(
        "25 30 0:23 / /data rw,relatime - ext4 /dev/sda1 rw\n",
        "31 30 0:24 / /etc/hosts rw,relatime - ext4 /dev/sda1 rw\n",
        "40 30 0:26 / /etc/xiaozhi/config.toml rw,relatime - ext4 /dev/sda1 rw\n",
    );
    assert!(
        is_persistent_in(mi, "/data/config.toml"),
        "目录挂载下的文件应持久"
    );
    assert!(
        is_persistent_in(mi, "/etc/xiaozhi/config.toml"),
        "文件级 bind mount 应持久"
    );
    assert!(
        !is_persistent_in(mi, "/etc/xiaozhi/other.toml"),
        "容器层文件不应判持久"
    );
    assert!(
        !is_persistent_in(mi, "/datax/config.toml"),
        "前缀相似但非子路径不应误判"
    );
}

/// revision（内容指纹）：同内容稳定、内容变则变、**敏感字段也参与**
/// （否则"只改了密钥"会被误判为没变，过期写入检查就形同虚设）。
#[test]
fn revision_tracks_content() {
    let a = full_test_config();
    assert_eq!(
        config_revision(&a),
        config_revision(&a.clone()),
        "同内容必须稳定"
    );

    let mut b = a.clone();
    b.llm.model = "another-model".into();
    assert_ne!(
        config_revision(&a),
        config_revision(&b),
        "内容变化必须改变指纹"
    );

    // 密钥：**值**有意不参与指纹（见 `config_revision`，避免指纹成为密钥的比对依据），
    // 但**有无**（presence）必须参与 —— 否则"删除密钥"这类改动不会被其他端察觉。
    let mut c = a.clone();
    c.llm.api_key = "sk-changed".into();
    assert_eq!(
        config_revision(&a),
        config_revision(&c),
        "密钥值不参与指纹（有意为之）"
    );
    c.llm.api_key = String::new();
    assert_ne!(
        config_revision(&a),
        config_revision(&c),
        "密钥有无（presence）必须改变指纹"
    );

    // 逐字段：任何一处默认值/字段变动都应被察觉（这里抽查 audio 与 aiui）
    let mut d = a.clone();
    d.audio.downlink_lead_ms += 1;
    assert_ne!(config_revision(&a), config_revision(&d));
    let mut e = a.clone();
    e.aiui.enabled = !e.aiui.enabled;
    assert_ne!(config_revision(&a), config_revision(&e));
}

/// 客户端原样回传整份 GET 结果（含 revision 包装字段）时，包装字段不得进入配置结构。
#[test]
fn wrapper_keys_are_stripped_before_persist() {
    let mut v = serde_json::json!({
        "llm": {"model": "m1"},
        "revision": "deadbeef",
        "expected_revision": "deadbeef",
    });
    strip_wrapper_keys(&mut v);
    assert!(v.get("revision").is_none());
    assert!(v.get("expected_revision").is_none());
    assert_eq!(v["llm"]["model"], "m1", "业务字段不受影响");
    // 剔除后仍是合法 Config（未知键本就被忽略，双保险）
    let _: Config = serde_json::from_value(v).unwrap();
}

// ---------- 密钥打码 / presence / 空串语义 ----------

/// GET 打码：非空密钥被**摘除**只留 presence 标记，空密钥标记为 `false`，非密钥字段不受影响。
#[test]
fn persist_config_roundtrip() {
    let dir = std::env::temp_dir().join(format!("xz_cfg_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("data.db");
    let cfg: Config = toml::from_str(
        r#"
[server]
port = 8000

[tts]
backend = "xfyun"

[tts.xfyun]
app_id = "a"
voice = "xiaoyan"
"#,
    )
    .unwrap();
    // 与 put_config 管线一致：写库前先 normalize（旧 `backend` 搬到 `engine`）
    let mut cfg = cfg;
    cfg.normalize();
    persist_config_file(path.to_str().unwrap(), &cfg).unwrap();
    let loaded = store_load(&path).unwrap();
    assert_eq!(loaded.server.port, 8000);
    assert_eq!(loaded.tts.xfyun.voice, "xiaoyan");
    // normalize 已把旧 `backend` 搬到规范位置 `engine`，值不丢
    assert_eq!(loaded.tts.engine, "xfyun");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 旧 schema 配置文件（含已废弃的 listen 键、P4 之前的 `[tts]` 扁平写法）应能静默解析
/// （未知键忽略），且规范化后 `backend`/扁平字段落到规范位置。
#[test]
fn legacy_config_with_listen_key_parses() {
    let legacy = r#"
[server]
listen = "192.168.99.250:8000"
expected_token = ""

[tts]
backend = "sherpa"
model = "/data/models/Kokoro/model.int8.onnx"
"#;
    let mut cfg: Config = toml::from_str(legacy).expect("旧 schema 配置应能解析");
    cfg.normalize();
    assert_eq!(cfg.server.port, 8000); // 缺省回退
    assert_eq!(cfg.tts.engine, "kokoro");
    assert_eq!(
        cfg.tts.kokoro.model, "/data/models/Kokoro/model.int8.onnx",
        "旧 [tts].model 必须搬到 [tts.kokoro]"
    );
}
