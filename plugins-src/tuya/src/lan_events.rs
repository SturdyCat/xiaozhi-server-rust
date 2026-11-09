//! S3f/S3h/S3i：LAN 域事件处理器（帧编解码 / 控制命令 / 会话协商）。
//!
//! 全部是纯函数证据面：无 I/O、无凭证；输入/解码错误也写结果通道
//! （ok:false），对拍方可见。协议语义在 `lan_codec`（0x55AA）/ `lan_frame_6699`
//! （0x6699）/ `lan_session`，本文件只做载荷 JSON ↔ 内参的搬运。宿主侧对拍见
//! `plugins/wasm/tests/tuya_lan.rs` 与 `tuya_lan_session.rs`（rustuya 本尊）。

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::xiaozhi::plugin::types;

use crate::lan_codec;
use crate::lan_frame_6699;
use crate::lan_session;

/// LAN 域结果通道前缀（宿主按 `<prefix>:<req_id>` 读回）。
pub(crate) const LAN_PREFIX: &str = "tuya-lan";
/// LAN 事件输入解码上限（解码后 16KB；结果 hex ×2 不超 kv 64KB 上限）。
const LAN_INPUT_CAP: usize = 16 * 1024;

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub(crate) fn hex_decode(hex: &str, cap: usize, what: &str) -> Result<Vec<u8>, String> {
    let hex = hex.trim();
    if hex.len() % 2 != 0 {
        return Err(format!("{what} hex 长度必须为偶数"));
    }
    if hex.len() / 2 > cap {
        return Err(format!(
            "{what} 解码后超限（{} > {cap} 字节）",
            hex.len() / 2
        ));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| format!("{what} hex 非法：{e}"))
        })
        .collect()
}

fn optional_hex(v: &Value, key: &str, cap: usize) -> Result<Option<Vec<u8>>, String> {
    match v.get(key).and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => Ok(Some(hex_decode(s, cap, key)?)),
        _ => Ok(None),
    }
}

/// 结果统一落通道：`Ok` 分支已含 ok 字段，`Err` 转成 `{"ok":false,"error":..}`。
pub(crate) fn lan_result(
    rid: &str,
    value: Result<Value, String>,
) -> Result<Vec<types::Effect>, String> {
    let effect = value.unwrap_or_else(|e| json!({"ok": false, "error": e}));
    Ok(vec![types::Effect::KvPut(types::EffectKvPut {
        key: format!("{LAN_PREFIX}:{rid}"),
        value: effect.to_string().into_bytes(),
    })])
}

pub(crate) fn payload_of(payload: &str) -> Result<Value, String> {
    serde_json::from_str::<Value>(payload).map_err(|e| format!("载荷非法：{e}"))
}

fn seqno_of(v: &Value) -> Result<u32, String> {
    v.get("seqno")
        .and_then(Value::as_u64)
        .map(|s| s as u32)
        .ok_or_else(|| "载荷缺少 seqno".to_string())
}

pub(crate) fn required_hex(v: &Value, key: &str, cap: usize) -> Result<Vec<u8>, String> {
    let s = v
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("载荷缺少 {key}"))?;
    hex_decode(s, cap, key)
}

// ------------------------------------------------------------------
// S3f/S3g：帧编解码（lan-pack / lan-parse）
// ------------------------------------------------------------------

pub(crate) fn lan_pack(rid: &str, payload: &str) -> Result<Vec<types::Effect>, String> {
    lan_result(rid, lan_pack_inner(payload))
}

fn lan_pack_inner(payload: &str) -> Result<Value, String> {
    let v = payload_of(payload)?;
    let seqno = seqno_of(&v)?;
    let cmd = v.get("cmd").and_then(Value::as_u64).ok_or("载荷缺少 cmd")? as u32;
    let body = hex_decode(
        v.get("payload_hex").and_then(Value::as_str).unwrap_or(""),
        LAN_INPUT_CAP,
        "payload",
    )?;
    let hmac_key = optional_hex(&v, "hmac_key_hex", 64)?;
    // S3g：format = "6699"（3.5/AES-GCM，需 key_hex 16B + iv_hex 12B + retcode?）；
    // 缺省 0x55AA（CRC / HMAC 变体）。
    if v.get("format").and_then(Value::as_str) == Some("6699") {
        let key = required_hex(&v, "key_hex", 16)?;
        let iv_hex = v
            .get("iv_hex")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or("6699 需要 iv_hex（12 字节）")?;
        let iv = hex_decode(iv_hex, 12, "iv")?;
        let retcode = v.get("retcode").and_then(Value::as_u64).map(|r| r as u32);
        let frame = lan_frame_6699::pack_frame_6699(seqno, cmd, &body, retcode, &key, &iv)?;
        return Ok(json!({"ok": true, "frame_hex": hex_encode(&frame)}));
    }
    let frame = lan_codec::pack_frame_55aa(seqno, cmd, &body, hmac_key.as_deref())?;
    Ok(json!({"ok": true, "frame_hex": hex_encode(&frame)}))
}

pub(crate) fn lan_parse(rid: &str, payload: &str) -> Result<Vec<types::Effect>, String> {
    lan_result(rid, lan_parse_inner(payload))
}

fn lan_parse_inner(payload: &str) -> Result<Value, String> {
    let v = payload_of(payload)?;
    let data = hex_decode(
        v.get("data_hex").and_then(Value::as_str).unwrap_or(""),
        LAN_INPUT_CAP,
        "data",
    )?;
    let hmac_key = optional_hex(&v, "hmac_key_hex", 64)?;
    if v.get("format").and_then(Value::as_str) == Some("6699") {
        let key = required_hex(&v, "key_hex", 16)?;
        return match lan_frame_6699::parse_frame_6699(&data, &key) {
            Ok(frame) => Ok(json!({
                "ok": true,
                "seqno": frame.seqno,
                "cmd": frame.cmd,
                "retcode": frame.retcode,
                "payload_hex": hex_encode(&frame.payload),
                "iv_hex": hex_encode(&frame.iv),
            })),
            Err(e) => Ok(json!({"ok": false, "error": e})),
        };
    }
    match lan_codec::parse_frame_55aa_strip(&data, hmac_key.as_deref()) {
        Ok(frame) => Ok(json!({
            "ok": true,
            "seqno": frame.seqno,
            "cmd": frame.cmd,
            "retcode": frame.retcode,
            "payload_hex": hex_encode(&frame.payload),
        })),
        Err(e) => Ok(json!({"ok": false, "error": e})),
    }
}

// ------------------------------------------------------------------
// S3h/S3i：控制命令帧（lan-command，version = "3.3" 缺省 / "3.4"）
// ------------------------------------------------------------------

pub(crate) fn lan_command(rid: &str, payload: &str) -> Result<Vec<types::Effect>, String> {
    lan_result(rid, lan_command_inner(payload))
}

fn lan_command_inner(payload: &str) -> Result<Value, String> {
    let v = payload_of(payload)?;
    let seqno = seqno_of(&v)?;
    let device_id = v
        .get("device_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("载荷缺少 device_id")?;
    let cid = v
        .get("cid")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let command = v
        .get("command")
        .and_then(Value::as_str)
        .ok_or("载荷缺少 command")?;
    let key = required_hex(&v, "key_hex", 16)?;
    let dps = v.get("dps").cloned().unwrap_or(Value::Null);
    // heartbeat 不需要 t（v3.4 载荷 strip uid/t；v3.3 不支持 heartbeat）。
    let t = v.get("t").and_then(Value::as_u64).unwrap_or(0);
    let version = v.get("version").and_then(Value::as_str).unwrap_or("3.3");
    let (cmd, frame) = match version {
        "3.3" => lan_codec::command_frame_33(seqno, device_id, cid, t, command, dps, &key)?,
        "3.4" => lan_codec::command_frame_34(seqno, device_id, cid, t, command, dps, &key)?,
        // S3j：v3.5 = 0x6699/GCM 帧级加密（载荷只拼 "3.5" 头，不做帧前加密）。
        "3.5" => {
            let iv_hex = v
                .get("iv_hex")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or("3.5 需要 iv_hex（12 字节）")?;
            let iv = hex_decode(iv_hex, 12, "iv")?;
            lan_frame_6699::command_frame_35(seqno, device_id, cid, t, command, dps, &key, &iv)?
        }
        other => return Err(format!("暂不支持的 version：{other}（3.3 / 3.4 / 3.5）")),
    };
    Ok(json!({
        "ok": true,
        "version": version,
        "cmd": cmd,
        "frame_hex": hex_encode(&frame),
    }))
}

// ------------------------------------------------------------------
// S3i：v3.4 会话协商（lan-session-start / lan-session-finish）
// ------------------------------------------------------------------

/// 会话第 1 步：SessKeyNegStart 完整帧。nonce/IV 由调用方传入（事件面纯函数；
/// 编排侧经宿主 `host-core.random` 取熵，见 `lan_session::random_nonce`）。
/// `version` 缺省 `"3.4"`（0x55AA HMAC 尾 + ECB 载荷）；`"3.5"` = 0x6699/GCM
/// （nonce 明文进帧，IV 必填）。
pub(crate) fn lan_session_start(rid: &str, payload: &str) -> Result<Vec<types::Effect>, String> {
    lan_result(rid, lan_session_start_inner(payload))
}

fn lan_session_start_inner(payload: &str) -> Result<Value, String> {
    let v = payload_of(payload)?;
    let seqno = seqno_of(&v)?;
    let local_key = required_hex(&v, "local_key_hex", 16)?;
    let nonce = required_hex(&v, "nonce_hex", 16)?;
    let (version, frame) = match session_version(&v)? {
        "3.4" => (
            "3.4",
            lan_session::session_start_frame(seqno, &local_key, &nonce)?,
        ),
        "3.5" => {
            let iv = required_hex(&v, "iv_hex", 12)?;
            (
                "3.5",
                lan_session::session_start_frame_35(seqno, &local_key, &nonce, &iv)?,
            )
        }
        other => return Err(format!("暂不支持的 version：{other}（3.4 / 3.5）")),
    };
    Ok(json!({
        "ok": true,
        "version": version,
        "cmd": lan_session::SESS_KEY_NEG_START,
        "frame_hex": hex_encode(&frame),
    }))
}

/// 会话事件 version 字段（缺省 3.4；lan-command 的 3.3 由其自身分派处理）。
fn session_version(v: &Value) -> Result<&str, String> {
    match v.get("version").and_then(Value::as_str).unwrap_or("3.4") {
        v @ ("3.4" | "3.5") => Ok(v),
        other => Err(format!("暂不支持的 version：{other}（3.4 / 3.5）")),
    }
}

/// 会话第 2+3 步：验证设备回包 → finish 帧 + 会话密钥 blob。
/// `resp_payload_hex` 是**解密后**的回包 payload（48 字节）——帧级解析用
/// lan-parse（retcode 启发式适用于 JSON 面；会话面直接吃 payload 对齐
/// rustuya `verify_session_key_response` 的入参）。
pub(crate) fn lan_session_finish(rid: &str, payload: &str) -> Result<Vec<types::Effect>, String> {
    lan_result(rid, lan_session_finish_inner(payload))
}

fn lan_session_finish_inner(payload: &str) -> Result<Value, String> {
    let v = payload_of(payload)?;
    let seqno = seqno_of(&v)?;
    let local_key = required_hex(&v, "local_key_hex", 16)?;
    let nonce = required_hex(&v, "nonce_hex", 16)?;
    let resp = required_hex(&v, "resp_payload_hex", LAN_INPUT_CAP)?;
    let (version, finish) = match session_version(&v)? {
        "3.4" => (
            "3.4",
            lan_session::session_finish(seqno, &local_key, &nonce, &resp)?,
        ),
        "3.5" => {
            let iv = required_hex(&v, "iv_hex", 12)?;
            (
                "3.5",
                lan_session::session_finish_35(seqno, &local_key, &nonce, &resp, &iv)?,
            )
        }
        other => return Err(format!("暂不支持的 version：{other}（3.4 / 3.5）")),
    };
    Ok(json!({
        "ok": true,
        "version": version,
        "cmd": lan_session::SESS_KEY_NEG_FINISH,
        "frame_hex": hex_encode(&finish.frame),
        "session_key_hex": hex_encode(&finish.session_key_blob),
        "remote_nonce_hex": hex_encode(&finish.remote_nonce),
    }))
}
