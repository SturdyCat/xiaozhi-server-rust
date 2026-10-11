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
    // 指令闸门：样例里默认关闭，且填的默认词表就是需求里那三个词
    assert!(!cfg.command.enabled);
    assert_eq!(cfg.command.keywords, vec!["退下", "闭嘴", "关闭"]);
    assert_eq!(
        cfg.command.match_mode,
        crate::plugins::command::MatchMode::Exact
    );
    // 长度闸门：样例里同样写明（漏写 = 用户不知道有这道保险），且默认值必须是 5
    assert_eq!(cfg.command.max_chars, 5);
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

/// 下行抖动缓冲护栏：设备解码队列是 **N 包 × 帧长**，`downlink_lead_ms` 超过它必须被
/// **收敛并告警**——节拍器开局预充 `ceil(lead/帧长)+1` 帧，固件队满即静默丢包
///（`PushPacketToDecodeQueue` 返回 false、零日志）= 回复开头少一截。
#[test]
fn audio_lead_ms_is_capped_to_device_queue_packets() {
    let mut a = AudioConfig::default();
    // 官方固件 20 包 × 60ms 帧 → 安全上限 (20−2)×60 = 1080ms
    assert_eq!((a.downlink_lead_ms, a.downlink_queue_packets), (1000, 20));
    assert_eq!(a.max_lead_ms(), 1080);
    // 超限 → 收敛到上限
    a.downlink_lead_ms = 1200;
    a.normalize();
    assert_eq!(a.downlink_lead_ms, 1080, "1200ms 开局预充 21 包 > 20 包，必须被收敛");
    // 合法值原样保留
    a.downlink_lead_ms = 600;
    a.normalize();
    assert_eq!(a.downlink_lead_ms, 600);
    // 默认值必须落在安全区内（否则每个新部署都在丢包）
    let d = AudioConfig::default();
    assert!(d.downlink_lead_ms <= d.max_lead_ms());
    // ⚠️ 上限随**帧长**变：队列是 N **包**，40ms 帧只能装 800ms → 上限 (20−2)×40 = 720
    let mut f = AudioConfig { downlink_frame_duration_ms: 40, downlink_lead_ms: 1000, ..AudioConfig::default() };
    assert_eq!(f.max_lead_ms(), 720, "40ms 帧的队容量是 20×40=800ms，不能按 1200ms 算");
    f.normalize();
    assert_eq!(f.downlink_lead_ms, 720, "40ms 帧下 1000ms 提前量会被静默丢包，必须被收敛");
    // 「要更多缓存」的唯一路径：自刷固件放大包数 → 同步 downlink_queue_packets → 上限自动放宽
    let mut big = AudioConfig { downlink_queue_packets: 66, downlink_lead_ms: 3000, ..AudioConfig::default() };
    assert_eq!(big.max_lead_ms(), 64 * 60);
    big.normalize();
    assert_eq!(big.downlink_lead_ms, 3000, "队列放大后 3000ms 提前量应被接受");
    // 帧长 0 非法（节拍器零等待 = 尽速灌入）→ 回落默认并保证上限仍合理
    let mut z = AudioConfig { downlink_frame_duration_ms: 0, downlink_lead_ms: 1000, ..AudioConfig::default() };
    z.normalize();
    assert_eq!(z.downlink_frame_duration_ms, 60, "0 帧长必须被修正");
}

/// 服务器侧水库护栏（`[audio].downlink_buffer_*`）：容量有上限、水库关闭则起跑闸一并归零、
/// 起跑闸不超过容量、最长等待 ≥ 起跑闸。三条不变式都直接决定"会不会把首帧无限期拖住"。
#[test]
fn audio_reservoir_params_are_normalized() {
    // 默认：容量 30s、起跑闸 1.5s、最长等待 4s（生产默认必须自洽）
    let d = AudioConfig::default();
    assert_eq!(
        (d.downlink_buffer_ms, d.downlink_buffer_start_ms, d.downlink_buffer_start_max_ms),
        (30_000, 1_500, 4_000)
    );
    assert!(d.downlink_buffer_start_ms <= d.downlink_buffer_ms);
    assert!(d.downlink_buffer_start_max_ms >= d.downlink_buffer_start_ms);

    // 容量有上限：手滑写 10^9 也不会把每会话内存吃光
    let mut big = AudioConfig { downlink_buffer_ms: 1_000_000, ..AudioConfig::default() };
    big.normalize();
    assert_eq!(big.downlink_buffer_ms, 180_000, "水库容量必须被收敛到上限");

    // 水库关闭（0）→ 起跑闸与最长等待一并归零（没有水库可攒）
    let mut off = AudioConfig {
        downlink_buffer_ms: 0,
        downlink_buffer_start_ms: 1_500,
        downlink_buffer_start_max_ms: 4_000,
        ..AudioConfig::default()
    };
    off.normalize();
    assert_eq!(
        (off.downlink_buffer_start_ms, off.downlink_buffer_start_max_ms),
        (0, 0),
        "水库关闭时起跑闸必须归零（否则起跑前白等到超时）"
    );

    // 起跑闸 > 容量 → 收敛到容量；最长等待 < 起跑闸 → 抬到起跑闸
    let mut odd = AudioConfig {
        downlink_buffer_ms: 2_000,
        downlink_buffer_start_ms: 9_000,
        downlink_buffer_start_max_ms: 10,
        ..AudioConfig::default()
    };
    odd.normalize();
    assert_eq!(odd.downlink_buffer_start_ms, 2_000, "攒不出比容量更多的音频");
    assert_eq!(odd.downlink_buffer_start_max_ms, 2_000, "最长等待不得小于起跑闸");
}

/// 下载缓冲预算（`AudioConfig::download_budget_ms`）：由水库容量派生并夹在 [1s, 10s]。
///
/// 为什么要夹：水库里是 Opus 帧（~2KB/s），下载缓冲里是 PCM f32（~64KB/s @16kHz，贵 ~30 倍）
/// ——两者等深会把每会话内存从几十 KB 抬到几 MB；而太小又会让"下载"退化回逐片等消费。
#[test]
fn download_budget_is_derived_and_bounded() {
    let base = AudioConfig { downlink_buffer_ms: 30_000, ..AudioConfig::default() };
    assert_eq!(base.download_budget_ms(), 10_000, "大水库下取上限 10s（内存口径）");
    let small = AudioConfig { downlink_buffer_ms: 2_000, ..AudioConfig::default() };
    assert_eq!(small.download_budget_ms(), 2_000, "小水库跟随水库容量");
    let off = AudioConfig { downlink_buffer_ms: 0, ..AudioConfig::default() };
    assert_eq!(off.download_budget_ms(), 1_000, "水库关闭也要留下限（解耦下载与编码）");
    // 派生值必须与水库容量脱钩地**恒**在界内：配置怎么改都不会把 PCM 内存顶飞
    for ms in [0, 1, 999, 1_000, 8_000, 60_000, 180_000] {
        let c = AudioConfig { downlink_buffer_ms: ms, ..AudioConfig::default() };
        assert!(
            (1_000..=10_000).contains(&c.download_budget_ms()),
            "download_budget_ms({ms}) 越界"
        );
    }
}
