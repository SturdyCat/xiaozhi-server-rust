//! Existing pure/preview services only. No device calls, memory writes, audio cache or sessions.
use super::contracts::StageResult;
use crate::config::Config;
use crate::media::media::{AudioCodec, AudioFormat, TerminalProfile};
use crate::workflow::session::SessionParams;
use serde_json::{json, Value};
use std::time::Instant;

pub(super) fn preview(cfg: &Config, action: &str, _text: &str) -> StageResult {
    let t = Instant::now();
    let mut r = StageResult::ok(action, 0, "configuration_preview");
    r.details = match action {
        "input.profile" => json!({"sample_rate":16000,"channels":1,"format":"pcm_s16le","max_seconds":15,"transport_negotiation_tested":false,"device_capture_tested":false,
            "terminal_defaults":[profile(cfg,AudioCodec::Opus),profile(cfg,AudioCodec::PcmS16Le)]}),
        "output.profile" => json!({"sample_rate":cfg.audio.downlink_sample_rate,"frame_duration_ms":cfg.audio.downlink_frame_duration_ms,"channels":cfg.audio.channels,"binary_protocol_version":cfg.audio.binary_protocol_version,
            "measurement":"configuration_only","socket_written":false,"played":false,"opus_verified":false,"playback":cfg.playback,
            "terminal_defaults":[profile(cfg,AudioCodec::Opus),profile(cfg,AudioCodec::PcmS16Le)]}),
        "wakeup.preview" => {
            if cfg.wakeup.validate().is_err() { return StageResult::error(action,"invalid_config","Invalid saved wakeup greeting configuration"); }
            json!({"enabled":cfg.wakeup.enabled,"active":cfg.wakeup.active(),"greetings":cfg.wakeup.pool(),"persist":cfg.wakeup.persist,"synthesis_tested":false})
        }
        #[cfg(feature = "native-plugins")]
        "soul.preview" => match crate::plugins::soul::preview::build(cfg,"<node-test>") {
            Ok(p) => { r.text = Some(p.instructions.clone()); p.to_json() }
            Err(_) => return StageResult::error(action,"invalid_config","Invalid saved soul preset or prompt variables"),
        },
        // pure-base 人格业务未编译：预览如实投影实际会发送的 instructions（= system_prompt）。
        #[cfg(not(feature = "native-plugins"))]
        "soul.preview" => {
            let Ok(instructions) = crate::plugins::soul::instructions(cfg, "<node-test>") else {
                return StageResult::error(action,"invalid_config","Invalid saved soul configuration");
            };
            r.text = Some(instructions.clone());
            json!({"instructions": instructions, "source": "llm.system_prompt"})
        }
        "memory.recall" | "memory.writeback.dry_run" => return StageResult::error(action,"unsupported_action","No snapshot-isolated read-only memory test adapter; use existing scoped memory preview API. This test never opens, migrates or writes a memory DB."),
        "tts.playback" => return StageResult::error(action,"unsupported_action","HTTP provider measurements cannot verify native playback; use the native diagnostic"),
        _ => return StageResult::error(action,"unsupported_action","Unknown test action_id"),
    };
    r.elapsed_ms = t.elapsed().as_millis() as u64;
    r
}
fn format(f: AudioFormat) -> Value {
    json!({"codec":match f.codec {AudioCodec::Opus=>"opus",AudioCodec::PcmS16Le=>"pcm_s16le"},"codec_version":f.codec_version,
        "sample_rate":f.sample_rate,"channels":f.channels,"frame_ms":f.frame_ms,"identity":f.identity()})
}
fn profile(cfg: &Config, codec: AudioCodec) -> Value {
    // The same pure builders are used by the actual session/transport paths.
    let params = SessionParams::configured_defaults(cfg, codec);
    // The core benchmark describes the generic session transport only. Device-specific
    // adapters provide their own diagnostics and are not part of the base build.
    let p = TerminalProfile::esp(&params);
    let requested_start =
        if cfg.playback.mode == crate::config::playback::PlaybackMode::Announcement {
            cfg.playback.announcement_start_ms
        } else {
            cfg.audio.downlink_buffer_start_ms
        };
    let queue_ms = None::<u64>;
    let queue_bytes = None::<usize>;
    json!({"terminal":"generic","scope":"configured_defaults_not_negotiated_device",
        "input":format(p.input),"output":format(p.output),
        "binary_protocol_version":json!(cfg.audio.binary_protocol_version),
        "binary_version_note":"Actual session binary version follows device hello, not this saved default",
        "capabilities":{"bounded_utterance":p.capabilities.bounded_utterance,"mcp":p.capabilities.mcp,"playback_ack":p.capabilities.playback_ack},
        "effective":{"lead_ms":params.downlink_lead_ms,"reservoir_ms":params.downlink_buffer_ms,"download_buffer_ms":params.download_buffer_ms,
            "start_ms":params.downlink_buffer_start_ms,"start_max_ms":params.downlink_buffer_start_max_ms},
        "queue":{"transport_ms":queue_ms,"transport_bytes":queue_bytes,"device_capacity_ms":null,"device_capacity_verified":false},
        "clamps":{"requested_start_ms":requested_start,"effective_start_ms":params.downlink_buffer_start_ms,
            "start_clamped":requested_start!=params.downlink_buffer_start_ms},
        "measurement":"configuration_only","negotiated":false,"socket_written":false,"played":false,"codec_verified":false})
}
