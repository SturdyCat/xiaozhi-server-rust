//! `config.rs` 的规范化（P4 配置约定统一）回归测试。
//!
//! 外移自 `config.rs`（`AGENTS.md` §5.10：测试一律独立文件）。
//! 覆盖：`engine` 键补全、旧别名映射、`[<cap>.<id>]` 私有段双读、**保存只写新结构**、
//! 以及"engine id 必须与注册表实现一致"的漂移护栏。

use super::*;
use crate::plugins::registry::{impl_id, impls_of, Capability, REGISTRY};

#[test]
fn canonical_engine_maps_aliases_and_defaults() {
    let specs = crate::plugins::tts::TTS_ENGINES;
    assert_eq!(canonical_engine("", specs, "kokoro"), "kokoro", "空 = 默认");
    assert_eq!(canonical_engine("  ", specs, "kokoro"), "kokoro");
    assert_eq!(canonical_engine("sherpa", specs, "kokoro"), "kokoro", "旧别名");
    assert_eq!(canonical_engine("SHERPA", specs, "kokoro"), "kokoro", "大小写不敏感");
    assert_eq!(canonical_engine("Xfyun", specs, "kokoro"), "xfyun");
    assert_eq!(
        canonical_engine("whisper", specs, "kokoro"),
        "whisper",
        "未知值原样返回（由 PluginHost 报错，不静默回退）"
    );
}

/// 默认配置也必须带 `engine` 键（否则 GET /api/config 会回一个空 engine，
/// 管理页下拉框选不中任何实现）。
#[test]
fn default_config_has_engine_keys_filled() {
    let cfg = Config::default();
    assert_eq!(cfg.asr.engine, "sensevoice");
    assert_eq!(cfg.vad.engine, "silero");
    assert_eq!(cfg.tts.engine, "kokoro");
    assert_eq!(cfg.llm.engine, "openai");
    assert_eq!(cfg.memory.engine_id(), "graph");
}

/// `normalize()` 幂等（load / default / merge 三条路径都可能重复调用）。
#[test]
fn normalize_is_idempotent() {
    let mut cfg = Config::default();
    let once = serde_json::to_value(&cfg).unwrap();
    cfg.normalize();
    assert_eq!(serde_json::to_value(&cfg).unwrap(), once);
}

/// **漂移护栏**：配置层声明的 engine id 必须与注册表的实现一一对应。
///
/// 这条把"文档/表格里的承诺"变成测试断言（本仓库的既有病是文档与代码漂移，见
/// `docs/plugin-architecture-unification.md` §7 风险 8）：改名描述符 id 却忘了改
/// `TTS_ENGINES` 之类，会在这里直接打红。
#[test]
fn declared_engine_ids_match_registry_implementations() {
    let tables: &[(Capability, &[EngineSpec])] = &[
        (Capability::Asr, crate::plugins::asr::ASR_ENGINES),
        (Capability::Vad, crate::plugins::vad::VAD_ENGINES),
        (Capability::Tts, crate::plugins::tts::TTS_ENGINES),
        (Capability::Llm, crate::plugins::llm::LLM_ENGINES),
    ];
    for (cap, specs) in tables {
        let registered: Vec<&str> = impls_of(*cap).map(|d| impl_id(d.id)).collect();
        for spec in specs.iter() {
            assert!(
                registered.contains(&spec.id),
                "能力 {} 声明了 engine id \"{}\"，但注册表里的实现是 {:?}——请同步",
                cap.id(),
                spec.id,
                registered
            );
            for alias in spec.aliases {
                assert!(
                    !registered.contains(alias),
                    "别名 \"{alias}\" 与某个实现 id 撞名（会造成二义）"
                );
            }
        }
        // 反向：每个实现都必须能被 engine 键选中（否则该实现永远不可达）
        for id in registered {
            assert!(
                specs.iter().any(|s| s.id == id),
                "能力 {} 的实现 \"{id}\" 没有对应的 engine 声明，配置永远选不中它",
                cap.id()
            );
        }
    }
}

/// 描述符 `fields_path` 也必须与约定一致：`[<cap>.<id>]` 或（单实现能力）`[<cap>]`。
#[test]
fn descriptor_fields_paths_follow_convention() {
    for d in REGISTRY {
        if d.fields.is_empty() {
            continue; // 无私有字段的能力（voices/firmware）
        }
        // FullChain 的配置段名是历史的 `[aiui]`（能力 id 为 `fullchain`）——唯一例外，显式列出。
        let section = match d.capability {
            Capability::FullChain => "aiui",
            c => c.id(),
        };
        assert_eq!(
            d.fields_path.first().copied(),
            Some(section),
            "{} 的 fields_path 必须以能力段开头",
            d.id
        );
        if d.fields_path.len() == 2 {
            assert_eq!(
                d.fields_path[1],
                impl_id(d.id),
                "{} 的私有段名必须 = 实现 id",
                d.id
            );
        }
    }
}

/// 仓库自带样例（`config.example.toml`）必须能解析并规范化——
/// 它是最常被直接复制的文件，范例写错等于每个新用户都踩一次。
#[test]
fn example_config_parses_and_normalizes() {
    let text = include_str!("../../config.example.toml");
    let mut cfg: Config = toml::from_str(text).expect("config.example.toml 必须可解析");
    cfg.normalize();
    assert_eq!(cfg.asr.engine, "sensevoice");
    assert_eq!(cfg.vad.engine, "silero");
    assert_eq!(cfg.tts.engine, "kokoro");
    assert_eq!(cfg.tts.kokoro.lang, "zh");
    assert_eq!(cfg.llm.engine, "openai");
    assert_eq!(cfg.memory.engine_id(), "graph");
    // 内置默认灵魂：样例里写了 preset，且它必须是真实存在的预设（写错样例 = 每个新用户踩一次）
    assert_eq!(cfg.soul.preset, crate::plugins::soul::presets::DEFAULT_ID);
    assert!(
        crate::plugins::soul::presets::get(&cfg.soul.preset).is_some(),
        "config.example.toml 的 preset 必须是已知预设"
    );
}

/// 回传/落盘形状：`[tts]` 只有 `engine` + 跨实现项 + `[tts.kokoro]`/`[tts.xfyun]` 子段，
/// 旧扁平键与 `backend` **永不出现**（否则客户端会以为新位置没生效）。
#[test]
fn serialized_config_has_no_legacy_tts_keys() {
    let cfg = Config::default();
    let v = serde_json::to_value(&cfg).unwrap();
    let tts = v["tts"].as_object().unwrap();
    assert_eq!(tts["engine"], "kokoro");
    assert!(tts.contains_key("kokoro"), "私有段必须在");
    for k in crate::plugins::tts::LEGACY_BODY_FIELDS {
        assert!(!tts.contains_key(*k), "旧键 {k} 不得出现在回包/落盘里");
    }
    assert!(!tts.contains_key("backend"));
    for k in ["speed", "speaker", "cache_entries"] {
        assert!(tts.contains_key(k), "跨实现参数 {k} 应留 `[tts]` 本体");
    }
    let text = toml::to_string_pretty(&cfg).unwrap();
    assert!(!text.contains("backend"), "{text}");
    assert!(text.contains("[tts.kokoro]"), "{text}");
}
