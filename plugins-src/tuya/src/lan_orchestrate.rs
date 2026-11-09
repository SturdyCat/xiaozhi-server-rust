//! S3m/S3n/S3o/S3p：Tuya LAN **会话步进**（握手回包 / 数据面 / 关闭收敛）。
//!
//! 建链与首帧在 `lan_connect`；心跳与 timer 分发在 `lan_heartbeat`；重连退避
//! 策略在 `lan_reconnect`；状态 kv 在 `lan_state`。本文件只做 `io.tcp.*` 回灌
//! 事件的相位步进：
//! - `WaitResp`：握手回包（流式重组，半包只入缓冲）→ 验证派生 → finish →
//!   `Established`；
//! - `Established`：流式帧切分 + 会话密钥解密 + 心跳应答识别；
//! - `io.tcp.closed`：相位收敛——`Established|WaitResp` 可退避重连则转
//!   `Reconnecting`，否则 `Closed`。
//!
//! 两代差异（S3o）：握手/数据帧布局 0x55AA HMAC 尾 vs 0x6699 帧级 GCM；v3.5
//! 每帧 IV 由 `lan_session::derive_iv` 从 nonce/blob + seqno 确定性派生。所有
//! 失败都先落状态 kv 再返回 Err（泵路径对业务性 Err 静默，kv 是唯一可观测面）。

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::xiaozhi::plugin::{host_core as host, host_net as net, types};

use crate::lan_events::{hex_decode, hex_encode, required_hex};
use crate::lan_reconnect;
use crate::lan_session;
use crate::lan_state::{find_state_by_conn, state_write};

/// 按会话版本选择流式切分器（布局差异见 `lan_stream` 模块注释）。
fn split_by_version(buf: &mut Vec<u8>, version: &str) -> Result<Vec<Vec<u8>>, String> {
    if version == "3.5" {
        crate::lan_stream::feed_split_v35(buf, &[])
    } else {
        crate::lan_stream::feed_split_55aa(buf, &[])
    }
}

/// `io.tcp.data` 步进：WaitResp 态收 SessKeyNegResp → 验证派生 → 发 finish →
/// Established（blob 落状态 kv）；Established 态做数据面处理——流式帧切分
/// （粘包/半包，缓冲持久化在状态 kv）+ 会话密钥解密 + 心跳应答识别。
pub(crate) fn on_tcp_data(
    payload: &[u8],
    correlation_id: Option<&str>,
) -> Result<Vec<types::Effect>, String> {
    let Some(conn_id) = correlation_id.and_then(|c| c.parse::<u64>().ok()) else {
        return Ok(vec![]); // 非本编排的连接（如监听端点入站），静默跳过
    };
    let Some((device_id, mut state)) = find_state_by_conn(conn_id)? else {
        return Ok(vec![]);
    };
    host::log(
        "debug",
        &format!(
            "io.tcp.data 步进：conn={conn_id} device={device_id} state={}",
            state.get("state").and_then(Value::as_str).unwrap_or("?")
        ),
    );
    let phase = state
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match phase.as_str() {
        "WaitResp" => {
            let version = state
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("3.4")
                .to_string();
            // 握手回包也走流式切分：半包到达时仅入持久缓冲、保持 WaitResp（S3o
            // v3.5 对拍故意把回包拆两次写）；完整帧才步进。
            let split = (|| -> Result<(Vec<Vec<u8>>, Vec<u8>), String> {
                let mut buf = hex_decode(
                    state
                        .get("rx_buf_hex")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    crate::lan_stream::MAX_RX_BUF * 2,
                    "rx_buf",
                )?;
                buf.extend_from_slice(payload);
                let frames = split_by_version(&mut buf, &version)?;
                Ok((frames, buf))
            })();
            let (frames, mut buf) = match split {
                Ok(v) => v,
                Err(e) => {
                    let msg = format!("会话握手收流失败（{device_id}）：{e}");
                    fail_state(&device_id, &mut state, &msg)?;
                    return Err(msg);
                }
            };
            if frames.is_empty() {
                state["rx_buf_hex"] = json!(hex_encode(&buf));
                state["last_writer"] = json!("waitresp-buf");
                state_write(&device_id, &state)?;
                return Ok(vec![]);
            }
            // 正常握手只有 1 帧；多余完整帧回填缓冲，交 Established 数据面处理。
            let wire = frames[0].clone();
            if frames.len() > 1 {
                let extra: Vec<u8> = frames[1..].concat();
                buf.splice(0..0, extra);
                host::log(
                    "warn",
                    &format!("设备 {device_id} 握手阶段收到多余帧，已回填缓冲"),
                );
            }
            let step = (|| -> Result<(), String> {
                let local_key = required_hex(&state, "local_key_hex", 16)?;
                let nonce = required_hex(&state, "nonce_hex", 16)?;
                let seqno = state.get("seqno").and_then(Value::as_u64).unwrap_or(2) as u32;
                let finish = if version == "3.5" {
                    let (_s, resp) = lan_session::wire_resp_to_payload_35(&wire, &local_key)?;
                    let iv = lan_session::derive_iv(&nonce, seqno, "finish");
                    lan_session::session_finish_35(seqno, &local_key, &nonce, &resp, &iv)?
                } else {
                    let (_s, resp) = lan_session::wire_resp_to_payload(&wire, &local_key)?;
                    lan_session::session_finish(seqno, &local_key, &nonce, &resp)?
                };
                net::net_send(conn_id, &finish.frame)
                    .map_err(|e| format!("SessKeyNegFinish 发送失败：{e}"))?;
                state["state"] = json!("Established");
                state["version"] = json!(version);
                state["session_key_hex"] = json!(hex_encode(&finish.session_key_blob));
                state["remote_nonce_hex"] = json!(hex_encode(&finish.remote_nonce));
                state["seqno"] = json!(seqno + 1);
                state["rx_buf_hex"] = json!(hex_encode(&buf));
                state["last_writer"] = json!("handshake");
                Ok(())
            })();
            match step {
                Ok(()) => {
                    lan_reconnect::mark_established(&mut state);
                    state_write(&device_id, &state)?;
                }
                Err(e) => {
                    let msg = format!("会话握手步进失败（{device_id}）：{e}");
                    fail_state(&device_id, &mut state, &msg)?;
                    return Err(msg);
                }
            }
        }
        "Established" => {
            // 流式帧切分（粘包/半包，缓冲持久化在状态 kv）+ 会话密钥解密 +
            // 心跳应答识别。设备主动上报（Status 等）暂计数，业务接线留后续。
            // 防御式「先落盘」：解析前先把入站字节并入持久缓冲，解析中途任何
            // 失败都不丢流（trap 会中断整个事件，写丢失的教训）。
            let step = (|| -> Result<(Vec<(u32, Vec<u8>)>, Vec<u8>), String> {
                let version = state
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or("3.4")
                    .to_string();
                let session_key = required_hex(&state, "session_key_hex", 16)?;
                let mut buf = hex_decode(
                    state
                        .get("rx_buf_hex")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    crate::lan_stream::MAX_RX_BUF * 2,
                    "rx_buf",
                )?;
                buf.extend_from_slice(payload);
                let frames = split_by_version(&mut buf, &version)?;
                let mut frames_out = Vec::new();
                for frame in &frames {
                    let decoded = if version == "3.5" {
                        crate::lan_stream::data_frame_decode_35(frame, &session_key)
                    } else {
                        crate::lan_stream::data_frame_decode(frame, &session_key)
                    };
                    let (cmd, plain) = decoded.map_err(|e| {
                        format!(
                            "{e}（帧长 {}，首 8B {:02x?}）",
                            frame.len(),
                            &frame[..frame.len().min(8)]
                        )
                    })?;
                    frames_out.push((cmd, plain));
                }
                Ok((frames_out, buf.clone()))
            })();
            let (frames_out, buf) = match step {
                Ok(v) => v,
                Err(e) => {
                    // 数据面单帧失败不断链：记状态、丢缓冲（防错位连锁），继续收流。
                    state["last_error"] = json!(e);
                    state["rx_buf_hex"] = json!("");
                    state_write(&device_id, &state)?;
                    host::log(
                        "warn",
                        &format!(
                            "会话 {device_id} 数据帧解析失败：{e}（入 {} 字节，首 8 字节 {:02x?}）",
                            payload.len(),
                            &payload[..payload.len().min(8)]
                        ),
                    );
                    return Ok(vec![]);
                }
            };
            let mut pongs = 0u64;
            let mut status_dps = 0usize;
            for (cmd, plain) in &frames_out {
                if *cmd == 0x09 {
                    pongs += 1;
                }
                // S3q：设备主动上报（Status/DpQuery/UpdateDps）→ DP 缓存 + 在线
                // 语义 + `tuya.official.lan-status` 事件；非上报命令不污染 DP。
                if let Some(n) = crate::lan_status::apply(&device_id, &mut state, *cmd, plain) {
                    status_dps += n;
                }
            }
            state["rx_buf_hex"] = json!(hex_encode(&buf));
            state["data_frames"] = json!(
                state
                    .get("data_frames")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    + frames_out.len() as u64
            );
            if pongs > 0 {
                state["last_pong_ms"] = json!(host::now_ms());
            }
            state["last_writer"] = json!("data");
            // 落盘（防御式流式处理的终点：解析成功才提交缓冲与计数）
            state_write(&device_id, &state)?;
            host::log(
                "debug",
                &format!(
                    "会话 {device_id} 数据帧 {} 个（心跳应答 {pongs}，状态 DP {status_dps}）",
                    frames_out.len()
                ),
            );
        }
        _ => {}
    }
    Ok(vec![])
}

/// 握手阶段失败收敛：策略允许 → `Reconnecting`（起退避 timer），否则
/// `Failed`；两种情况都落状态 kv（结果通道与 kv 双可观测面）。
fn fail_state(device_id: &str, state: &mut Value, msg: &str) -> Result<(), String> {
    state["state"] = json!("Failed");
    state["error"] = json!(msg);
    state["last_writer"] = json!("handshake-fail");
    let scheduled = lan_reconnect::schedule(state, msg)?;
    if scheduled {
        host::log(
            "info",
            &format!("设备 {device_id} 握手失败，进入退避重连：{msg}"),
        );
    }
    state_write(device_id, state)
}

/// `io.tcp.closed`：`Established|WaitResp` 的活跃连接关闭 → 有策略则
/// `Reconnecting`（S3p 自动重连），否则 `Closed`。其余相位（`Reconnecting`
/// 退避在途 / `Failed` / `Closed`）忽略，防重复计数。
pub(crate) fn on_tcp_closed(correlation_id: Option<&str>) -> Result<Vec<types::Effect>, String> {
    let Some(conn_id) = correlation_id.and_then(|c| c.parse::<u64>().ok()) else {
        return Ok(vec![]);
    };
    let Some((device_id, mut state)) = find_state_by_conn(conn_id)? else {
        return Ok(vec![]);
    };
    let phase = state
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if phase != "Established" && phase != "WaitResp" {
        return Ok(vec![]);
    }
    let reason = format!("连接关闭（phase={phase}）");
    let scheduled = lan_reconnect::schedule(&mut state, &reason)?;
    if scheduled {
        host::log("info", &format!("设备 {device_id} {reason}，进入退避重连"));
    } else {
        state["state"] = json!("Closed");
    }
    state["last_writer"] = json!("closed");
    state_write(&device_id, &state)?;
    Ok(vec![])
}
