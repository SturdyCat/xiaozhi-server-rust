//! 注册表各实现的**保存期校验**（错误文案必须含「怎么修」）。
//!
//! 从 `impls.rs` 拆出（AGENTS.md §5.10 文件规模约定，纯搬移、行为不变）：
//! 描述符表（选中/签名/构建）留在 `impls.rs`，校验文案集中在本文件，
//! 两边经 `pub(super)` 可见性连接——子模块互访不需放宽为 `pub`。
//!
//! 文案口径：说清"缺什么 + 怎么修"，多个问题尽量一条一句话说完。

use anyhow::Result;

use crate::config::Config;

// ---- 校验（错误文案必须含「怎么修」）----

pub(super) fn validate_asr(cfg: &Config) -> Result<()> {
    if !cfg!(feature = "sherpa") {
        anyhow::bail!(
            "本二进制未启用 `sherpa` feature：SenseVoice ASR 需要 --features sherpa 编译（Docker 镜像已内置）。"
        );
    }
    if cfg.asr.model.trim().is_empty() {
        anyhow::bail!(
            "[asr].model 未配置：请填 SenseVoice INT8 模型路径（如 /data/models/SenseVoiceSmall/model.int8.onnx）。\
             容器内缺失模型时入口脚本会自动下载；离线环境请先预置到挂载卷。"
        );
    }
    Ok(())
}

pub(super) fn validate_vad(cfg: &Config) -> Result<()> {
    // 仅 sherpa 构建需要真实模型（未启用 feature 时用占位实现，见 plugins/vad/mod.rs）
    #[cfg(feature = "sherpa")]
    if cfg.vad.model.trim().is_empty() {
        anyhow::bail!(
            "[vad].model 未配置：请填 Silero VAD 模型路径（如 /data/models/silero_vad.onnx）。\
             容器内缺失模型时入口脚本会自动下载；离线环境请先预置到挂载卷。"
        );
    }
    let _ = cfg;
    Ok(())
}

pub(super) fn validate_tts_xfyun(cfg: &Config) -> Result<()> {
    let x = &cfg.tts.xfyun;
    if x.app_id.trim().is_empty() || x.api_key.trim().is_empty() || x.api_secret.trim().is_empty() {
        anyhow::bail!(
            "[tts].engine=\"xfyun\" 需要 [tts.xfyun] 的 app_id / api_key / api_secret（讯飞开放平台「在线语音合成」控制台获取）。"
        );
    }
    Ok(())
}

/// Kokoro 的硬条件：`[tts.kokoro]` 的模型/音色数据/词表路径必须非空
/// （P4 前这些字段在 `[tts]` 本体，`normalize()` 已负责搬家）。
pub(super) fn validate_tts_kokoro(cfg: &Config) -> Result<()> {
    if !cfg!(feature = "sherpa") {
        anyhow::bail!(
            "本二进制未启用 `sherpa` feature：本地 Kokoro TTS 需要 --features sherpa 编译；\
             或把 [tts].engine 切到在线服务商（如 xfyun）。"
        );
    }
    let k = &cfg.tts.kokoro;
    let missing: Vec<&str> = [
        ("model", k.model.as_str()),
        ("voices", k.voices.as_str()),
        ("tokens", k.tokens.as_str()),
        ("data_dir", k.data_dir.as_str()),
    ]
    .iter()
    .filter(|(_, v)| v.trim().is_empty())
    .map(|(n, _)| *n)
    .collect();
    if !missing.is_empty() {
        anyhow::bail!(
            "[tts.kokoro] 缺少必要路径 {:?}：请填 Kokoro INT8 模型包内的对应文件路径\
             （默认 /data/models/Kokoro/…；容器内缺失模型时入口脚本会自动下载）。若配置写在旧位置 \
             `[tts].model`，保存一次即自动迁移到 `[tts.kokoro]`。",
            missing
        );
    }
    Ok(())
}

pub(super) fn validate_llm(cfg: &Config) -> Result<()> {
    // 硬条件：地址缺失无法调用（保留既有行为：密钥为空**不**阻止启动，见 warn_llm）
    if cfg.llm.api_base.trim().is_empty() {
        anyhow::bail!(
            "[llm].api_base 未配置：请填 OpenAI 兼容 **Responses API** 地址（如 https://api.example.com/v1/responses）。"
        );
    }
    Ok(())
}

/// LLM 软性问题：密钥为空 —— 服务照常启动，但每次对话都会在上游 401/403。
/// 状态上报为 `Degraded`（而不是静默），且热刷新时**不会**用空密钥覆盖正在工作的引擎。
pub(super) fn warn_llm(cfg: &Config) -> Option<String> {
    if cfg.llm.api_key.trim().is_empty() {
        Some(
            "[llm].api_key 为空：每次对话都会在上游报 401/403；在管理页填入密钥即可（新会话生效）"
                .into(),
        )
    } else {
        None
    }
}

pub(super) fn validate_aiui(cfg: &Config) -> Result<()> {
    if cfg.aiui.enabled
        && (cfg.aiui.appid.trim().is_empty()
            || cfg.aiui.api_key.trim().is_empty()
            || cfg.aiui.api_secret.trim().is_empty())
    {
        anyhow::bail!(
            "[aiui].enabled 需要 appid / api_key / api_secret（AIUI 平台应用三要素）；\
             不想用全链路模式就把 enabled 改回 false。"
        );
    }
    Ok(())
}

pub(super) fn validate_memory(cfg: &Config) -> Result<()> {
    cfg.memory.validate()
}

/// 记忆的软性问题：**没有 embedding**（当前为本地词法召回）、抽取路由缺凭据等。
/// 这些都不是错误 —— 记忆照常工作，只是能力受限，必须让 UI 能说出来（对齐 DSH 的
/// 「`enabled` 与实际激活态分开上报」），否则用户只会得到"开了没用"的结论。
pub(super) fn warn_memory(cfg: &Config) -> Option<String> {
    cfg.memory.warning(&cfg.llm)
}

pub(super) fn validate_soul(cfg: &Config) -> Result<()> {
    cfg.soul.validate()
}

/// 指令闸门的硬条件：启用时词表不得为空。
/// （词表为空 = 闸门不拦任何话——不致命但必须能被 `/api/plugins` 说成 `Degraded`，
/// 否则用户会得到"配了没用"的静默失败。）
pub(super) fn validate_command(cfg: &Config) -> Result<()> {
    cfg.command.validate()
}

/// `[wakeup]` 校验：段内自洽（[`crate::plugins::wakeup::WakeConfig::validate`]）
/// **加上**一处跨段矛盾检查。
///
/// 跨段那条是刻意的：持久化复用靠"内存 TTS 缓存承载成品帧"实现（合成完 → 从缓存取出 → 落盘），
/// 所以 `[tts].cache_entries = 0` 时**落盘会静默失效**（写不进去、下次还得重新合成）。
/// 静默失效是本仓库最忌讳的失败模式，因此在这里直接拦下并说清修法。
pub(super) fn validate_wake(cfg: &Config) -> Result<()> {
    cfg.wakeup.validate()?;
    if cfg.wakeup.active() && cfg.wakeup.persist && cfg.tts.cache_entries == 0 {
        anyhow::bail!(
            "[wakeup].persist = true 但 [tts].cache_entries = 0：持久化复用依赖内存缓存承载\
             成品帧，缓存被关掉时音频**无法落盘**（每次唤醒都会重新合成）。\
             请把 [tts].cache_entries 设为大于 0（推荐 256），或把 [wakeup].persist 改为 false。"
        );
    }
    Ok(())
}

/// NLU 的硬条件：启用且声明了意图时，意图必须有模板（否则永远命中不了 = 静默失效）；
/// 其余（技能名唯一性、action.kind、模型阈值范围）统一在 [`crate::plugins::skill::validate`]。
pub(super) fn validate_nlu(cfg: &Config) -> Result<()> {
    crate::plugins::skill::validate(&cfg.command, &cfg.nlu)?;
    if cfg.nlu.enabled {
        for it in &cfg.nlu.intents {
            if it.templates.is_empty() {
                anyhow::bail!(
                    "[nlu] 意图「{}」已声明但 templates 为空：请至少填一个模板（如 [\"把{{room}}的灯{{action}}\"]），\
                     或暂时把 [nlu].enabled 改回 false。",
                    it.name
                );
            }
        }
    }
    // TinyBERT 语义分类（可选第二级）：开关打开时三路径必填且文件必须存在，阈值在 0~1。
    // 构建失败只会在会话启动时 warn 回落，这里前置拦截配置错误。
    if cfg.nlu.bert_enabled {
        for (key, path) in [
            ("bert_model", &cfg.nlu.bert_model),
            ("bert_vocab", &cfg.nlu.bert_vocab),
            ("bert_labels", &cfg.nlu.bert_labels),
        ] {
            if path.trim().is_empty() {
                anyhow::bail!(
                    "[nlu].bert_enabled = true 但 {key} 为空：请填写 ONNX 模型/词表/标签文件路径，                     或把 [nlu].bert_enabled 改回 false。"
                );
            }
            if !std::path::Path::new(path).is_file() {
                anyhow::bail!(
                    "[nlu].{key} = 「{path}」不是存在的文件：请确认路径（相对路径以服务端工作目录为基准），                     或把 [nlu].bert_enabled 改回 false。"
                );
            }
        }
        if !(0.0..=1.0).contains(&cfg.nlu.bert_threshold) {
            anyhow::bail!(
                "[nlu].bert_threshold = {} 超出范围：请设为 0~1（建议 0.75），或恢复默认值。",
                cfg.nlu.bert_threshold
            );
        }
    }
    Ok(())
}
