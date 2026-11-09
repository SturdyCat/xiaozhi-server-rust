//! Implementation signatures used for in-memory refresh comparisons.

use crate::config::Config;

/// 远端引擎签名：引擎 id 即全部输入（utterance 级桥无本地构建参数）。
pub(super) fn sig_asr_wasm(cfg: &Config) -> String {
    format!("wasm-remote|{}", cfg.asr.engine_id())
}

// ---- 签名（仅内存比较）----

pub(super) fn sig_asr(cfg: &Config) -> String {
    let a = &cfg.asr;
    format!(
        "{}|{}|{}|{}|{}|{}",
        a.model, a.tokens, a.language, a.use_itn, a.num_threads, a.provider
    )
}
pub(super) fn sig_vad(cfg: &Config) -> String {
    let v = &cfg.vad;
    format!(
        "{}|{}|{}|{}",
        v.model, v.threshold, v.min_silence_duration, v.min_speech_duration
    )
}
pub(super) fn sig_vad_wasm_policy(cfg: &Config) -> String {
    format!("wasm-policy|{}", sig_vad(cfg))
}
/// 覆盖 Kokoro 的全部构建输入；JSON 长度/转义避免路径中分隔符引发签名碰撞。
pub(super) fn sig_tts_kokoro(cfg: &Config) -> String {
    format!(
        "kokoro|{}",
        serde_json::to_string(&cfg.tts.kokoro).expect("Kokoro 配置可序列化")
    )
}
/// ⚠️ 含密钥，**绝不打日志**（仅内存比较）。
pub(super) fn sig_tts_xfyun(cfg: &Config) -> String {
    let x = &cfg.tts.xfyun;
    format!(
        "xfyun|{}|{}|{}|{}",
        x.app_id, x.api_key, x.api_secret, x.voice
    )
}
/// 策略包装的身份包含底层 Kokoro 构建输入；凭据/路径只用于内存比较，绝不打日志。
pub(super) fn sig_tts_wasm_policy(cfg: &Config) -> String {
    format!(
        "wasm-policy|{}",
        serde_json::to_string(&cfg.tts.kokoro).expect("Kokoro 配置可序列化")
    )
}
pub(super) fn sig_llm(cfg: &Config) -> String {
    let l = &cfg.llm;
    format!(
        "{}|{}|{}|{}|{}|{}",
        l.api_base, l.api_key, l.model, l.temperature, l.stream, l.system_prompt
    )
}
pub(super) fn sig_llm_wasm_policy(cfg: &Config) -> String {
    format!("wasm-policy|{}", sig_llm(cfg))
}
#[cfg(feature = "tuya-plugin")]
pub(super) fn sig_tuya(cfg: &Config) -> String {
    format!("{}|{}", cfg.tuya.engine_id(), cfg.tuya.enabled)
}
#[cfg(feature = "echomuse-plugin")]
pub(super) fn sig_echomuse(cfg: &Config) -> String {
    format!("{}|{}", cfg.echomuse.engine_id(), cfg.echomuse.enabled)
}
#[cfg(feature = "esp_home")]
pub(super) fn sig_esp_home(cfg: &Config) -> String {
    format!("{}|{}", cfg.esp_home.engine_id(), cfg.esp_home.enabled)
}
pub(super) fn sig_aiui(cfg: &Config) -> String {
    let a = &cfg.aiui;
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        a.enabled,
        a.appid,
        a.api_key,
        a.api_secret,
        a.scene,
        a.sn_prefix,
        a.voice,
        a.speed,
        a.volume,
        a.pitch,
        a.prompt,
        a.pace_ms
    )
}

pub(super) fn sig_memory(cfg: &Config) -> String {
    cfg.memory.signature()
}
pub(super) fn sig_soul(cfg: &Config) -> String {
    cfg.soul.signature()
}
/// 指令闸门无共享实例（每会话按快照构造 `CommandGate`），签名只用于状态上报。
pub(super) fn sig_command(cfg: &Config) -> String {
    cfg.command.signature()
}

pub(super) fn sig_wake(cfg: &Config) -> String {
    cfg.wakeup.signature()
}
/// 目录/动作plan/槽位值表全部入签；不再遗漏只改动作或条件的保存。
pub(super) fn sig_nlu(cfg: &Config) -> String {
    let mut value = serde_json::to_value(&cfg.nlu).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        object.remove("model_threshold");
        object.remove("model_margin");
    }
    format!("{}|{}", value, cfg.command.signature())
}
