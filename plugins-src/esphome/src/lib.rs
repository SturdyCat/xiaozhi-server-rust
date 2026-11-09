// esphome.official：ESPHome 桥实体目录策略 + Native API 明文/加密帧编解码。
//
// 阶段三 ESPHome 第一切片（S3a）：宿主在桥启动期派发 `connector.esp-home.prepare`，
// 本插件决定「哪些实体对外暴露 + 显示名覆盖」这一有界控制面决策。默认策略是
// **全放行直通**（逐实体 `exposed=true`、不改名）——与 native 引擎行为逐字节一致，
// 用户可换自建插件实现隐藏/改名（宿主逐索引校验，越界/超长显式拒绝）。
//
// S3k：`connector.esp-home.frame-pack` / `frame-parse` 事件承接 **明文帧编解码 +
// 核心消息 protobuf 编解码**（纯函数证据面；宿主对拍用 esphome-native-api 真连接）。
// S3t：`connector.esp-home.noise` 事件承接 **Noise NNpsk0 加密握手 + 传输帧**
// （ClientHello/ServerHello/msg1/msg2 与 AEAD 传输帧；状态在插件隔离 kv）——
// 加密协议语义与明文帧同属插件侧，宿主只搬字节。
// S3u：`connector.esp-home.session` 事件承接 **设备侧会话语义**——握手之后的消息
// 派发（Hello/DeviceInfo/Ping/ListEntities/SubscribeStates/Disconnect）与实体面应答，
// 会话状态同样只留插件隔离 kv。状态采集/音量命令执行/BLE 代理仍是 native 数据面
// （P8c 真机回归后逐域收口）。

#[global_allocator]
static ALLOC: guest_alloc::GrowingBump = guest_alloc::GrowingBump::new();

// S3k：帧/消息编解码单源（规格权威 = esphome-native-api frame/parser/proto）。
mod api_frame;
// S3r：服务端面消息（实体目录/状态/控制应答）protobuf 编解码（同一规格权威）。
mod api_catalog;
// S3t：Noise NNpsk0 加密握手（协议原语 / 线格式 / 会话状态机 / 事件面）。
mod hex;
mod noise;
mod noise_crypto;
mod noise_event;
mod noise_wire;
// S3u：设备侧会话（responder）——握手之后的消息派发与实体面应答。
mod session;
mod session_catalog;
mod session_event;

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::builder::{ManifestBuilder, RegistrationBuilder};
use xiaozhi_plugin_sdk::exports::lifecycle::Guest;
use xiaozhi_plugin_sdk::xiaozhi::plugin::{host_core as host, types};

use crate::hex::{hex_decode, hex_encode};

/// 插件 id：必须与上传请求 id、manifest.toml 登记项完全一致。
const PLUGIN_ID: &str = "esphome.official";
/// 结果通道前缀：宿主按 `<prefix>:<req_id>` 读回本插件的决策。
const RESULT_PREFIX: &str = "esphome-result";
/// 帧编解码结果通道前缀（S3k：帧 parity 证据面）。
const FRAME_PREFIX: &str = "esphome-frame";
/// 实体目录上限（与宿主侧校验同口径；超限直接业务性失败）。
const MAX_ENTITIES: usize = 64;

struct Component;

/// 从开放形状载荷中提取 req_id（宿主为每次调用生成，结果必须写回同一 id）。
fn req_id_of(payload: &str) -> String {
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|v| v.get("req_id").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// 要求 req_id 存在；缺失时终止本次决策，宿主显式失败（fail-closed）。
fn checked_req_id(payload: &str) -> Result<String, String> {
    let rid = req_id_of(payload);
    if rid.is_empty() {
        return Err("缺少 req_id".to_string());
    }
    Ok(rid)
}

/// 统一结果效果：把有界决策 JSON 写进 `<prefix>:<req_id>`。
fn result(value: &Value, req_id: &str) -> types::Effect {
    types::Effect::KvPut(types::EffectKvPut {
        key: format!("{}:{}", RESULT_PREFIX, req_id),
        value: value.to_string().into_bytes(),
    })
}

impl Guest for Component {
    fn manifest() -> types::PluginManifest {
        ManifestBuilder::new(PLUGIN_ID, "ESPHome 桥实体目录策略")
            .expect("静态构建：id/title 已内联校验")
            .version("0.5.0")
            .description(
                "connector.esp-home 扩展点：实体放行/显示名决策 + 明文/加密帧编解码 + 设备侧会话",
            )
            .permission("kv")
            .expect("kv 权限")
            .implements("connector.esp-home")
            .expect("扩展点")
            .official(true)
            .build()
            .expect("静态构建")
    }

    fn register() -> types::Registration {
        RegistrationBuilder::new()
            .menu_entry(
                "nav.main",
                "esphome-policy",
                "ESPHome 目录策略",
                "插件",
                116,
                "settings",
            )
            .expect("静态构建")
            .action("ping", "连通性自检", false, "返回插件 id、扩展点与契约版本")
            .expect("静态构建")
            .build()
    }

    fn handle_event(ev: types::Event) -> Result<Vec<types::Effect>, String> {
        let event = ev.topic.clone();
        let payload = String::from_utf8_lossy(&ev.payload).to_string();
        host::log("debug", &format!("{} event={}", PLUGIN_ID, event));
        match event.as_str() {
            "connector.esp-home.prepare" => {
                let rid = checked_req_id(&payload)?;
                let v: Value =
                    serde_json::from_str(&payload).map_err(|e| format!("载荷非法：{e}"))?;
                let entities = v
                    .get("entities")
                    .and_then(Value::as_array)
                    .ok_or("载荷缺少 entities 数组")?;
                if entities.len() > MAX_ENTITIES {
                    return Err(format!(
                        "实体目录超限（{} > {MAX_ENTITIES}）",
                        entities.len()
                    ));
                }
                // 默认策略：全放行直通。只透传索引与「放行」，不解释 object_id/kind，
                // 也不改显示名（name 缺省 = 宿主保留静态名）。
                let decisions: Vec<Value> = entities
                    .iter()
                    .filter_map(|e| e.get("index").and_then(Value::as_u64))
                    .map(|index| json!({"index": index, "exposed": true}))
                    .collect();
                Ok(vec![result(
                    &json!({"route": "native", "entities": decisions}),
                    &rid,
                )])
            }
            // S3k：明文帧编解码（纯函数证据面；esphome-native-api 真连接对拍）。
            "connector.esp-home.frame-pack" => {
                let rid = checked_req_id(&payload)?;
                frame_pack(&rid, &payload)
            }
            "connector.esp-home.frame-parse" => {
                let rid = checked_req_id(&payload)?;
                frame_parse(&rid, &payload)
            }
            // S3t：noise 加密握手 / 传输帧（状态在插件隔离 kv，事件间无内存态）。
            "connector.esp-home.noise" => noise_event::handle(&payload),
            // S3u：设备侧会话（responder）——握手后消息派发 + 实体面应答。
            "connector.esp-home.session" => session_event::handle(&payload),
            // 会话观察者挂钩：接受但不干预，避免产生未知事件噪音。
            e if e.starts_with("hook.") => Ok(vec![]),
            other => Err(format!("未知事件：{other}")),
        }
    }

    fn handle_service(
        service: String,
        method: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        // 未声明服务的插件一律显式拒绝（宿主 call-service 会拿到这条消息）
        Err(format!(
            "{PLUGIN_ID} 未提供服务 {service}.{method}（{} 字节载荷）",
            payload.len()
        ))
    }

    fn handle_action(action_id: String, payload: String) -> Result<String, String> {
        match action_id.as_str() {
            "ping" => Ok(format!(
                "插件={} 扩展点=connector.esp-home 契约=0.2 参数={}",
                PLUGIN_ID, payload
            )),
            other => Err(format!("{PLUGIN_ID} 未知动作：{other}")),
        }
    }

    fn self_test() -> Result<Vec<types::TestResult>, String> {
        let rid = req_id_of(r#"{"req_id":"selftest"}"#);
        let payload: Value = serde_json::from_str(
            r#"{"req_id":"selftest","entities":[{"index":0,"object_id":"x","name":"x","kind":"binary_sensor","writable":false}]}"#,
        )
        .map_err(|e| format!("自检载荷非法：{e}"))?;
        let count = payload
            .get("entities")
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        let noise = noise::self_check();
        let session = session::self_check();
        Ok(vec![
            types::TestResult {
                name: "req-id-and-catalog-parse".to_string(),
                ok: rid == "selftest" && count == 1,
                detail: format!(
                    "req_id={} 结果通道={RESULT_PREFIX}:<req_id> 实体数={count}",
                    rid
                ),
            },
            types::TestResult {
                name: "noise-nnpsk0-handshake".to_string(),
                ok: noise.is_ok(),
                detail: match &noise {
                    Ok(()) => "双角色握手 + 双向传输帧 + 错误 psk MAC 失败全通过".to_string(),
                    Err(e) => format!("失败：{e}"),
                },
            },
            types::TestResult {
                name: "esphome-device-session".to_string(),
                ok: session.is_ok(),
                detail: match &session {
                    Ok(()) => "明文会话 Hello/目录/状态/断开全链通过".to_string(),
                    Err(e) => format!("失败：{e}"),
                },
            },
        ])
    }
}

xiaozhi_plugin_sdk::export!(Component);

// ==================================================================
// S3k：明文帧编解码事件（纯函数，无 I/O）——错误写结果通道（ok:false），
// 与 tuya.official 的 LAN 编解码事件同口径。
// ==================================================================

fn frame_result(rid: &str, value: Result<Value, String>) -> Result<Vec<types::Effect>, String> {
    let effect = value.unwrap_or_else(|e| json!({"ok": false, "error": e}));
    Ok(vec![types::Effect::KvPut(types::EffectKvPut {
        key: format!("{FRAME_PREFIX}:{rid}"),
        value: effect.to_string().into_bytes(),
    })])
}

/// 核心消息 → protobuf 载荷（明文帧 / 加密传输帧共用单源；prost 语义：声明序 +
/// 默认值跳过）。`message` 取 `api_frame::message_type_id` 收录的子集；
/// hello_request / switch_command_request 由 guest 手写 protobuf，空载荷消息
/// （ping/disconnect 等）= 空 `Vec`。
pub(crate) fn message_payload(name: &str, v: &Value) -> Result<Vec<u8>, String> {
    let num_u32 = |key: &str| {
        v.get(key)
            .and_then(Value::as_u64)
            .map(|n| n as u32)
            .unwrap_or(0)
    };
    let num_f32 = |key: &str| v.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let text = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let payload_bytes = match name {
        "hello_request" => {
            let client_info = v
                .get("client_info")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let major = v
                .get("api_version_major")
                .and_then(Value::as_u64)
                .unwrap_or(1) as u32;
            let minor = v
                .get("api_version_minor")
                .and_then(Value::as_u64)
                .unwrap_or(10) as u32;
            api_frame::hello_request_payload(client_info, major, minor)
        }
        "switch_command_request" => {
            let key = v.get("key").and_then(Value::as_u64).unwrap_or(0) as u32;
            let state = v.get("state").and_then(Value::as_bool).unwrap_or(false);
            let device_id = v.get("device_id").and_then(Value::as_u64).unwrap_or(0) as u32;
            api_frame::switch_command_payload(key, state, device_id)
        }
        // S3r：服务端面消息（桥的实体目录 / 状态 / 应答）与 number 命令对称编码。
        "hello_response" => api_catalog::hello_response_payload(
            num_u32("api_version_major"),
            num_u32("api_version_minor"),
            &text("server_info"),
            &text("name"),
        ),
        "device_info_response" => api_catalog::device_info_response_payload(
            &text("name"),
            &text("mac_address"),
            &text("esphome_version"),
            &text("model"),
            &text("manufacturer"),
            &text("friendly_name"),
            &text("suggested_area"),
            v.get("api_encryption_supported")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            num_u32("legacy_bluetooth_proxy_version"),
            num_u32("bluetooth_proxy_feature_flags"),
        ),
        "list_entities_binary_sensor_response" => api_catalog::list_entities_binary_sensor_payload(
            &text("object_id"),
            num_u32("key"),
            &text("name"),
            &text("device_class"),
        ),
        "list_entities_number_response" => api_catalog::list_entities_number_payload(
            &text("object_id"),
            num_u32("key"),
            &text("name"),
            num_f32("min_value"),
            num_f32("max_value"),
            num_f32("step"),
            &text("unit_of_measurement"),
            num_u32("mode"),
        ),
        "binary_sensor_state_response" => api_catalog::binary_sensor_state_payload(
            num_u32("key"),
            v.get("state").and_then(Value::as_bool).unwrap_or(false),
            v.get("missing_state")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            num_u32("device_id"),
        ),
        "number_state_response" => api_catalog::number_state_payload(
            num_u32("key"),
            num_f32("state"),
            v.get("missing_state")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            num_u32("device_id"),
        ),
        "number_command_request" => api_catalog::number_command_payload(
            num_u32("key"),
            num_f32("state"),
            num_u32("device_id"),
        ),
        _ => Vec::new(), // 空载荷消息（ping/disconnect/device_info/list_entities 等）
    };
    Ok(payload_bytes)
}

/// 核心消息 → 明文帧。
fn frame_pack_inner(payload: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_str(payload).map_err(|e| format!("载荷非法：{e}"))?;
    let name = v
        .get("message")
        .and_then(Value::as_str)
        .ok_or("载荷缺少 message")?;
    let message_type = api_frame::message_type_id(name)?;
    let payload_bytes = message_payload(name, &v)?;
    let frame = api_frame::frame_encode(message_type, &payload_bytes)?;
    Ok(json!({
        "ok": true,
        "message": name,
        "type": message_type,
        "frame_hex": hex_encode(&frame),
    }))
}

fn frame_pack(rid: &str, payload: &str) -> Result<Vec<types::Effect>, String> {
    frame_result(rid, frame_pack_inner(payload))
}

/// 按消息面解码已解析载荷（明文帧 `frame-parse` 与加密传输帧 `noise open` 共用单源）。
/// 已知客户端 → 服务端消息解出字段；未知消息原样透传；坏 wire 显式 Err。
pub(crate) fn attach_decoded(
    message_type: u8,
    payload: &[u8],
    out: &mut Value,
) -> Result<(), String> {
    if message_type == 2 {
        if let Ok(info) = api_frame::hello_response_decode(payload) {
            out["hello"] = json!({
                "api_version_major": info.api_version_major,
                "api_version_minor": info.api_version_minor,
                "server_info": info.server_info,
                "name": info.name,
            });
        }
    }
    // S3r：客户端 → 服务端消息按消息面解码（未知字段跳过；坏 wire 显式 ok:false）。
    match message_type {
        1 => {
            let info = api_catalog::hello_request_decode(payload)?;
            out["hello_request"] = json!({
                "client_info": info.client_info,
                "api_version_major": info.api_version_major,
                "api_version_minor": info.api_version_minor,
            });
        }
        51 => {
            let command = api_catalog::number_command_decode(payload)?;
            out["number_command"] = json!({
                "key": command.key,
                "state": command.state,
                "device_id": command.device_id,
            });
        }
        33 => {
            let command = api_catalog::switch_command_decode(payload)?;
            out["switch_command"] = json!({
                "key": command.key,
                "state": command.state,
                "device_id": command.device_id,
            });
        }
        _ => {}
    }
    Ok(())
}

/// 明文帧 → 类型 + 载荷（载荷 hex 透传）；已知消息额外解出关键字段。
fn frame_parse_inner(payload: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_str(payload).map_err(|e| format!("载荷非法：{e}"))?;
    let data_hex = v
        .get("data_hex")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let data = hex_decode(data_hex, api_frame::INPUT_CAP, "data")?;
    let frame = api_frame::frame_decode(&data)?;
    let mut out = json!({
        "ok": true,
        "type": frame.message_type,
        "message": api_frame::message_type_name(frame.message_type),
        "payload_hex": hex_encode(&frame.payload),
    });
    attach_decoded(frame.message_type, &frame.payload, &mut out)?;
    Ok(out)
}

fn frame_parse(rid: &str, payload: &str) -> Result<Vec<types::Effect>, String> {
    frame_result(rid, frame_parse_inner(payload))
}
