//! `plugins/tts/mod.rs` 的配置约定回归测试（P4）。
//!
//! 外移自 `tts/mod.rs`（`AGENTS.md` §5.10：测试一律独立文件）。
//! 覆盖：`engine`/`backend` 双读、旧扁平字段 → `[tts.kokoro]` 的搬家规则（含"新位置优先"）、
//! 以及**保存形态只写新结构**。

use super::*;

fn legacy_toml(text: &str) -> TtsConfig {
    toml::from_str::<TtsConfig>(text).expect("旧写法必须仍可解析")
}

#[test]
fn engine_key_prefers_new_value_and_falls_back_to_legacy_backend() {
    let mut new = legacy_toml(r#"engine = "XFYUN""#);
    assert_eq!(new.engine_id(), "xfyun", "大小写不敏感");
    new.normalize();
    assert_eq!(new.engine, "xfyun");

    let mut legacy = legacy_toml(r#"backend = "sherpa""#);
    assert_eq!(legacy.engine_id(), "kokoro", "旧 backend=sherpa 映射为 kokoro");
    legacy.normalize();
    assert_eq!(legacy.engine, "kokoro", "规范化后写回 engine 键");

    // 两者同时出现：新键优先（旧键只是兼容读入）
    let mut both = legacy_toml("engine = \"xfyun\"\nbackend = \"sherpa\"\n");
    assert_eq!(both.engine_id(), "xfyun");
    both.normalize();
    assert_eq!(both.engine, "xfyun");

    // 都未写：默认 kokoro
    let mut none = legacy_toml("");
    assert_eq!(none.engine_id(), "kokoro");
    none.normalize();
    assert_eq!(none.engine, "kokoro");
}

#[test]
fn unknown_engine_is_preserved_for_diagnosis() {
    let cfg = legacy_toml(r#"engine = "azure""#);
    assert_eq!(
        cfg.engine_id(),
        "azure",
        "未知值不得静默回退默认（否则用户以为配置生效了）"
    );
}

#[test]
fn legacy_flat_fields_move_into_kokoro_section() {
    let mut cfg = legacy_toml(
        r#"
backend = "sherpa"
model = "/m/k.onnx"
voices = "/m/voices.bin"
tokens = "/m/tokens.txt"
data_dir = "/m/data"
dict_dir = "/m/dict"
lexicon = "/m/a.txt"
lang = "en"
num_threads = 2
speed = 1.25
speaker = 3
cache_entries = 77
"#,
    );
    cfg.normalize();
    assert_eq!(cfg.kokoro.model, "/m/k.onnx");
    assert_eq!(cfg.kokoro.lang, "en");
    assert_eq!(cfg.kokoro.num_threads, 2);
    // 跨实现参数留在本体（不搬家）
    assert_eq!(cfg.speed, 1.25);
    assert_eq!(cfg.speaker, 3);
    assert_eq!(cfg.cache_entries, 77);

    // 保存形态：旧键不写回
    let text = toml::to_string_pretty(&cfg).unwrap();
    assert!(text.contains("[kokoro]"), "{text}");
    assert!(!text.contains("backend"), "{text}");
    assert!(!text.contains("model = \"/m/k.onnx\"\nlang"), "旧位置不得写入: {text}");
}

/// **新位置优先**：`[tts.kokoro]` 已有非默认值时不搬旧位置（避免旧键覆盖新键）。
#[test]
fn kokoro_section_wins_over_legacy_flat_fields() {
    let mut cfg = legacy_toml(
        r#"
model = "/old/model.onnx"
lang = "en"

[kokoro]
model = "/new/model.onnx"
"#,
    );
    cfg.normalize();
    assert_eq!(cfg.kokoro.model, "/new/model.onnx");
    // 子段里"仍是默认值"的字段才接受旧位置的值
    assert_eq!(cfg.kokoro.lang, "en");
}

/// 旧位置的值恰好等于默认值时不算"用户设置"（不产生迁移噪音，且结果等价）。
#[test]
fn legacy_explicit_defaults_are_not_considered_moves() {
    let mut cfg = legacy_toml(r#"model = "/data/models/Kokoro/model.int8.onnx""#);
    let before = cfg.kokoro.clone();
    cfg.normalize();
    assert_eq!(cfg.kokoro, before, "与默认值相同的旧字段搬家结果是等价的");
    assert_eq!(cfg.kokoro.model, "/data/models/Kokoro/model.int8.onnx");
}

/// 跨实现字段（`speed`/`speaker`/`cache_entries`）在旧、新写法下位置一致，必须都认。
#[test]
fn cross_impl_fields_stay_in_body() {
    let cfg = legacy_toml(
        r#"
speed = 0.9
speaker = 7
cache_entries = 12
"#,
    );
    assert_eq!(cfg.speed, 0.9);
    assert_eq!(cfg.speaker, 7);
    assert_eq!(cfg.cache_entries, 12);
}
