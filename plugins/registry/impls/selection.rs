//! Implementation selection predicates.

use crate::config::Config;

// ---- 选中判定（P4：统一按 `engine` 键 = 描述符 id 去掉 "<cap>." 前缀）----

pub(super) fn sel_always(_cfg: &Config) -> bool {
    true
}
pub(super) fn sel_asr(cfg: &Config) -> bool {
    cfg.asr.engine_id() == "sensevoice"
}

pub(super) fn sel_asr_wasm(cfg: &Config) -> bool {
    cfg.asr.engine_id() == "wasm-remote"
}

pub(super) fn sel_vad(cfg: &Config) -> bool {
    cfg.vad.engine_id() == "silero"
}
pub(super) fn sel_vad_wasm_policy(cfg: &Config) -> bool {
    cfg.vad.engine_id() == "wasm-policy"
}
pub(super) fn sel_tts_kokoro(cfg: &Config) -> bool {
    cfg.tts.engine_id() == "kokoro"
}
pub(super) fn sel_tts_xfyun(cfg: &Config) -> bool {
    cfg.tts.engine_id() == "xfyun"
}
pub(super) fn sel_tts_wasm_policy(cfg: &Config) -> bool {
    cfg.tts.engine_id() == "wasm-policy"
}
pub(super) fn sel_llm(cfg: &Config) -> bool {
    cfg.llm.engine_id() == "openai"
}
pub(super) fn sel_llm_wasm_policy(cfg: &Config) -> bool {
    cfg.llm.engine_id() == "wasm-policy"
}
#[cfg(feature = "tuya-plugin")]
pub(super) fn sel_tuya_native(cfg: &Config) -> bool {
    cfg.tuya.enabled && cfg.tuya.engine_id() == "native"
}
#[cfg(feature = "tuya-plugin")]
pub(super) fn sel_tuya_wasm_policy(cfg: &Config) -> bool {
    cfg.tuya.enabled && cfg.tuya.engine_id() == "wasm-policy"
}
#[cfg(feature = "echomuse-plugin")]
pub(super) fn sel_echomuse_native(cfg: &Config) -> bool {
    cfg.echomuse.enabled && cfg.echomuse.engine_id() == "native"
}
#[cfg(feature = "echomuse-plugin")]
pub(super) fn sel_echomuse_wasm_policy(cfg: &Config) -> bool {
    cfg.echomuse.enabled && cfg.echomuse.engine_id() == "wasm-policy"
}
#[cfg(feature = "esp_home")]
pub(super) fn sel_esp_home_native(cfg: &Config) -> bool {
    cfg.esp_home.enabled && cfg.esp_home.engine_id() == "native"
}
#[cfg(feature = "esp_home")]
pub(super) fn sel_esp_home_wasm_policy(cfg: &Config) -> bool {
    cfg.esp_home.enabled && cfg.esp_home.engine_id() == "wasm-policy"
}
#[cfg(feature = "esp_home")]
pub(super) fn sel_esp_home_wasm_session(cfg: &Config) -> bool {
    cfg.esp_home.enabled && cfg.esp_home.engine_id() == "wasm-session"
}
pub(super) fn sel_aiui(cfg: &Config) -> bool {
    cfg.aiui.enabled
}
/// P1-7 多后端：`selected` 按实现拆分（不拆的话 active() 对 graph/store 都为真，
/// 两条 memory 描述符会同时选中，`active_impl` 无法唯一确定）。
pub(super) fn sel_memory_graph(cfg: &Config) -> bool {
    cfg.memory.enabled && cfg.memory.engine == crate::plugins::memory::MemoryBackend::Graph
}
pub(super) fn sel_memory_store(cfg: &Config) -> bool {
    cfg.memory.enabled && cfg.memory.engine == crate::plugins::memory::MemoryBackend::Store
}

pub(super) fn sel_memory_rag(cfg: &Config) -> bool {
    cfg.memory.enabled && cfg.memory.engine == crate::plugins::memory::MemoryBackend::Rag
}
pub(super) fn sel_soul(cfg: &Config) -> bool {
    cfg.soul.enabled
}
pub(super) fn sel_command(cfg: &Config) -> bool {
    cfg.command.enabled
}
pub(super) fn sel_wake(cfg: &Config) -> bool {
    cfg.wakeup.enabled
}
pub(super) fn sel_nlu(cfg: &Config) -> bool {
    cfg.nlu.enabled
}
