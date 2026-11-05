//! 注册表的**字段元数据表**（能力公共字段 + 各实现私有字段）。
//!
//! 纯数据表：拆出原因见 `AGENTS.md` §5.10（文件规模约定）。这些常量是 `pub(super)`——
//! 只有同目录的 `mod.rs` / `impls.rs` 需要它们；对外仍经 `schema_json()` 投影。

use super::{Capability, FieldKind, FieldSchema, HotReload};

/// 能力级**公共**字段（跨实现共享，如 `[tts].engine` 选择器与 `cache_entries`）。
/// 键相对能力段（`["tts"]`），与实现的私有字段分开，避免同一字段在两个实现里重复声明。
///
/// P4 约定：每个能力都用 `engine` 选实现（值 = 描述符 id 去掉 `"<cap>."` 前缀）；
/// `[memory].engine` 的定义在 `fields_context.rs`（它是闭枚举 `graph|none`）。
pub(super) const CAPABILITY_COMMON_FIELDS: &[(
    Capability,
    &'static [&'static str],
    &'static [FieldSchema],
)] = &[
    (
        Capability::Asr,
        &["asr"],
        &[
            FieldSchema {
                key: "engine",
                label: "识别引擎",
                kind: FieldKind::Enum(&["sensevoice", "wasm-remote"]),
                default_hint: "sensevoice",
                help: "选择 `[asr]` 的实现：sensevoice = 本地离线（需模型文件）；\n\
                       wasm-remote = 远端引擎变体通道（`asr.provider` 扩展点插件，utterance 级；\n\
                       需 wasm-plugins feature 且插件已装载，缺依赖显式报错不静默回退）。改后需重启。",
                hot: HotReload::RestartOnly,
                required: true,
            },
            FieldSchema {
                key: "max_concurrent",
                label: "并发推理路上限",
                kind: FieldKind::Int,
                default_hint: "1",
                help: "跨会话的 ASR 推理闸门（排队背压）：本地引擎每路占 num_threads 线程，\
                        默认 1 路恰等于线程预算；算力富余可放宽。改后需重启。",
                hot: HotReload::RestartOnly,
                required: false,
            },
        ],
    ),
    (
        Capability::Vad,
        &["vad"],
        &[FieldSchema {
            key: "engine",
            label: "VAD 引擎",
            kind: FieldKind::Enum(&["silero", "wasm-policy"]),
            default_hint: "silero",
            help: "选择 `[vad]` 的实现：silero = native 帧级 VAD；wasm-policy = WASM 会话级控制策略 + native Silero（不传输 PCM）。改后新会话生效。",
            hot: HotReload::NextSession,
            required: true,
        }],
    ),
    (
        Capability::Tts,
        &["tts"],
        &[
            FieldSchema {
                key: "engine",
                label: "引擎 / 服务商",
                kind: FieldKind::Enum(&["kokoro", "xfyun", "wasm-policy"]),
                default_hint: "kokoro",
                help: "选择 `[tts]` 的实现（旧值 `sherpa` 仍可读）；保存后新会话生效。",
                hot: HotReload::NextSession,
                required: true,
            },
            FieldSchema {
                key: "cache_entries",
                label: "结果缓存条目数",
                kind: FieldKind::Int,
                default_hint: "256",
                help: "按句缓存已编码下行音频（跨会话共享），命中即免合成延迟；0 = 关闭。",
                hot: HotReload::RestartOnly,
                required: false,
            },
            FieldSchema {
                key: "speed",
                label: "语速",
                kind: FieldKind::Float,
                default_hint: "1.0",
                help: "跨实现参数（`TtsEngine::synthesize_stream` 入参）；1.0 为常速，语音助手不建议 >1.2。",
                hot: HotReload::NextSession,
                required: false,
            },
            FieldSchema {
                key: "speaker",
                label: "发音人 id",
                kind: FieldKind::Int,
                default_hint: "0",
                help: "跨实现的参数槽：Kokoro 用音色 id（测试台可临时覆盖），在线服务商忽略它。",
                hot: HotReload::NextSession,
                required: false,
            },
            FieldSchema {
                key: "max_concurrent",
                label: "并发合成路上限",
                kind: FieldKind::Int,
                default_hint: "1",
                help: "跨会话的 TTS 合成闸门（排队背压）：本地引擎每路占 num_threads 线程，\
                        默认 1 路恰等于线程预算；在线引擎（如讯飞）或算力富余可放宽。\
                        随 TTS 热切换对账，新会话生效。",
                hot: HotReload::NextSession,
                required: false,
            },
        ],
    ),
    (
        Capability::Llm,
        &["llm"],
        &[
            FieldSchema {
                key: "enabled",
                label: "启用 LLM",
                kind: FieldKind::Bool,
                default_hint: "true",
                help: "false = 回声模式：技能级联照常，未命中时不调云端 LLM，\
                        ASR 文本原样 TTS 回放（测试 ASR→TTS 链路用）。保存后下一轮生效。\
                        纯回声直通（级联也不拦截）：把 [nlu].enabled 与 [command].enabled 一并关闭。",
                hot: HotReload::Live,
                required: false,
            },
            FieldSchema {
                key: "engine",
                label: "LLM 实现",
                kind: FieldKind::Enum(&["openai", "wasm-policy"]),
                default_hint: "openai",
                help: "选择 `[llm]` 的实现：openai = 原生 Responses API；wasm-policy = WASM 控制策略 + 原生 OpenAI 数据面（需 wasm-plugins 与策略插件）。改后新会话生效。",
                hot: HotReload::NextSession,
                required: true,
            },
        ],
    ),
    (
        Capability::Tuya,
        &["tuya"],
        &[
            FieldSchema {
                key: "enabled",
                label: "启用涂鸦集成",
                kind: FieldKind::Bool,
                default_hint: "true",
                help: "关闭后不做云同步、局域网轮询或管理面操作；保存即生效。",
                hot: HotReload::Live,
                required: false,
            },
            FieldSchema {
                key: "engine",
                label: "连接器实现",
                kind: FieldKind::Enum(&["native", "wasm-policy"]),
                default_hint: "native",
                help: "native = 原生 rustuya 局域网数据面；wasm-policy = WASM 控制策略 + 原生数据面。旧值 backend/lan/wasm 仍可读，保存即生效。",
                hot: HotReload::Live,
                required: true,
            },
            FieldSchema {
                key: "region",
                label: "云区域",
                kind: FieldKind::Enum(&["cn", "us", "eu", "in"]),
                default_hint: "cn",
                help: "Tuya OpenAPI 区域；仅用于云目录同步，局域网控制不依赖云。",
                hot: HotReload::Live,
                required: false,
            },
            FieldSchema {
                key: "access_id",
                label: "云 access id",
                kind: FieldKind::Str,
                default_hint: "",
                help: "Tuya IoT 云开发项目 access id；留空则跳过云同步。",
                hot: HotReload::Live,
                required: false,
            },
            FieldSchema {
                key: "access_secret",
                label: "云 access secret",
                kind: FieldKind::Password,
                default_hint: "",
                help: "Tuya IoT 云开发项目 secret；仅写入，管理面不回显。",
                hot: HotReload::Live,
                required: false,
            },
            FieldSchema {
                key: "sync_interval_secs",
                label: "云同步周期（秒）",
                kind: FieldKind::Int,
                default_hint: "86400",
                help: "默认每日同步；0 = 仅手动。显式小于 60 秒会收敛到 60 秒。",
                hot: HotReload::Live,
                required: false,
            },
        ],
    ),
    (
        Capability::Echomuse,
        &["echomuse"],
        &[
            FieldSchema {
                key: "enabled",
                label: "启用 EchoMuse",
                kind: FieldKind::Bool,
                default_hint: "true",
                help: "启用 EchoMuse 设备控制面、mDNS 与管理入口；关闭后不挂载设备路由。需重启生效。",
                hot: HotReload::RestartOnly,
                required: false,
            },
            FieldSchema {
                key: "engine",
                label: "设备入口实现",
                kind: FieldKind::Enum(&["native", "wasm-policy"]),
                default_hint: "native",
                help: "native = 原生 EchoMuse 控制面；wasm-policy = WASM 控制策略元数据 + 原生设备面。旧值 wasm 仍可读。需重启生效。",
                hot: HotReload::RestartOnly,
                required: true,
            },
            FieldSchema {
                key: "token",
                label: "设备 token",
                kind: FieldKind::Password,
                default_hint: "",
                help: "设备 X-Em-Token；留空表示不校验，管理页不回显。需重启生效。",
                hot: HotReload::RestartOnly,
                required: false,
            },
            FieldSchema {
                key: "ble_proxy",
                label: "蓝牙代理",
                kind: FieldKind::Bool,
                default_hint: "false",
                help: "启用 EchoMuse BLE 代理能力协商与管理端点。需重启生效。",
                hot: HotReload::RestartOnly,
                required: false,
            },
            FieldSchema {
                key: "plain_port",
                label: "控制器专用端口",
                kind: FieldKind::Int,
                default_hint: "8767",
                help: "官方 EchoMuse controller 明文端口；0 表示仅主端口。需重启生效。",
                hot: HotReload::RestartOnly,
                required: false,
            },
            FieldSchema {
                key: "welcome",
                label: "设备接入欢迎语",
                kind: FieldKind::Text,
                default_hint: "我已连接，喊我的唤醒词或按一下按键就能开始对话。",
                help: "设备会话建立后播放一次；留空表示关闭。需重启生效。",
                hot: HotReload::RestartOnly,
                required: false,
            },
            FieldSchema {
                key: "music_dir",
                label: "音乐目录",
                kind: FieldKind::Dir,
                default_hint: "/data/music",
                help: "音乐面允许读取的根目录；留空关闭音乐面。需重启生效。",
                hot: HotReload::RestartOnly,
                required: false,
            },
        ],
    ),
    (
        Capability::FullChain,
        &["aiui"],
        &[FieldSchema {
            key: "enabled",
            label: "启用全链路",
            kind: FieldKind::Bool,
            default_hint: "false",
            help: "启用后本地 ASR/LLM/TTS 全部闲置（云端闭环计费）；需 appid/api_key/api_secret。",
            hot: HotReload::RestartOnly,
            required: true,
        }],
    ),
];
