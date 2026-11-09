//! S3u：ESPHome **设备侧会话语义**（responder 角色）——握手之后的消息派发与实体面应答。
//!
//! 本模块是「native `esp_home` 桥的连接内状态机」在插件侧的等价实现，纯状态机、
//! 无 I/O（字节搬运由宿主 `net` 服务承载，见 S3t 的 `connector.esp-home.noise` 衔接）：
//!
//! 1. `open`（[`Session::new_plaintext`] / [`Session::new_encrypted`]）：加密模式先由
//!    宿主发 ServerHello，客户端回 ClientHello+msg1 → msg2；
//! 2. [`Session::feed`]：对端字节 → 解密/解帧 → 逐消息派发并回包——
//!    `HelloRequest`→`HelloResponse`、`DeviceInfoRequest`→`DeviceInfoResponse`、
//!    `PingRequest`→`PingResponse`、`ListEntitiesRequest`→逐实体定义 + Done、
//!    `SubscribeStatesRequest`→全量状态快照并置订阅态、`DisconnectRequest`→应答并关闭；
//!    `NumberCommandRequest` / `SwitchCommandRequest` 解码后交给调用方（数据面仍 native）；
//! 3. [`Session::apply_states`]：订阅态下推送状态变化 → 对应实体的状态响应帧。
//!
//! 会话状态（身份 / 实体目录 / 订阅态 / 加密握手状态 / 流式缓冲）JSON 序列化，
//! 由 `session_event` 落插件隔离 kv——事件间无内存态，与 tuya LAN 编排同纪律。

use serde_json::{json, Value};

use crate::api_catalog;
use crate::api_frame::{self, next_frame_from_buffer};
use crate::hex::hex_encode;
use crate::noise::{Handshake, Phase, Role};
use crate::session_catalog::*;

/// 单次接收缓冲上限（防御性；会话正常远小于此）。
const MAX_RX_BUF: usize = 64 * 1024;
/// 会话侧 `server_info`（HelloResponse 字段；native crate 默认值的插件等价物）。
pub(crate) const SERVER_INFO: &str = "xiaozhi:esphome-plugin";
/// 设备侧 API 版本（与 native 桥一致：1.10）。
pub(crate) const API_VERSION_MAJOR: u32 = 1;
pub(crate) const API_VERSION_MINOR: u32 = 10;

/// 传输模式：明文（无 psk）或 Noise NNpsk0 加密。
pub(crate) enum Mode {
    Plaintext,
    Encrypted(Handshake),
}

/// 会话（设备侧 responder）。
pub(crate) struct Session {
    mode: Mode,
    /// 校验并归一化后的身份（name/mac/friendly_name/model/... 见 [`validate_identity`]）。
    identity: Value,
    /// 校验并归一化后的实体目录（见 [`validate_entities`]）。
    entities: Value,
    subscribed: bool,
    closed: bool,
    /// 明文模式的流式接收缓冲（加密模式的缓冲在 [`Handshake`] 内）。
    rx_buf: Vec<u8>,
}

/// 一次 [`Session::feed`] 的产出。
pub(crate) struct FeedOutcome {
    /// 需要写回对端的帧字节（握手帧 / 加密封装后的应答帧）。
    pub out: Vec<u8>,
    /// 对端消息解码结果（类型 / 名称 / 载荷 hex / 已知消息字段）。
    pub messages: Vec<Value>,
    pub phase: String,
    pub subscribed: bool,
    pub closed: bool,
}

impl Session {
    /// 明文会话（设备未配置 noise_psk；仅可信局域网）。
    pub(crate) fn new_plaintext(identity: Value, entities: Value) -> Self {
        Self {
            mode: Mode::Plaintext,
            identity,
            entities,
            subscribed: false,
            closed: false,
            rx_buf: Vec::new(),
        }
    }

    /// 加密会话：临时密钥必须由调用方（宿主 `host-core.random`）提供。
    pub(crate) fn new_encrypted(
        identity: Value,
        entities: Value,
        psk: [u8; crate::noise::PSK_LEN],
        ephemeral: [u8; crate::noise_crypto::DH_LEN],
    ) -> Self {
        Self {
            mode: Mode::Encrypted(Handshake::start(Role::Responder, psk, ephemeral)),
            identity,
            entities,
            subscribed: false,
            closed: false,
            rx_buf: Vec::new(),
        }
    }

    pub(crate) fn mode_str(&self) -> &'static str {
        match self.mode {
            Mode::Plaintext => "plaintext",
            Mode::Encrypted(_) => "encrypted",
        }
    }

    pub(crate) fn phase(&self) -> &'static str {
        match &self.mode {
            Mode::Plaintext => "ready",
            Mode::Encrypted(handshake) => handshake.phase().as_str(),
        }
    }

    pub(crate) fn subscribed(&self) -> bool {
        self.subscribed
    }

    /// 身份字段（`name` 等，供 ServerHello / HelloResponse 使用）。
    pub(crate) fn identity_str(&self, key: &str) -> &str {
        self.identity
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    /// 喂入对端原始字节（可半包/粘包），推进握手并派发已解出的完整消息。
    pub(crate) fn feed(&mut self, data: &[u8]) -> Result<FeedOutcome, String> {
        if self.closed {
            return Err("会话已关闭".to_string());
        }
        let mut out = Vec::new();
        let mut incoming: Vec<(u16, Vec<u8>)> = Vec::new();
        match &mut self.mode {
            Mode::Plaintext => {
                if self.rx_buf.len() + data.len() > MAX_RX_BUF {
                    return Err(format!("明文会话接收缓冲超限（> {MAX_RX_BUF}）"));
                }
                self.rx_buf.extend_from_slice(data);
                let mut frames = Vec::new();
                while let Some(frame) = next_frame_from_buffer(&mut self.rx_buf)? {
                    frames.push(frame);
                }
                for frame in frames {
                    let parsed = api_frame::frame_decode(&frame)?;
                    incoming.push((u16::from(parsed.message_type), parsed.payload));
                }
            }
            Mode::Encrypted(handshake) => {
                if handshake.phase() != Phase::Ready {
                    out.extend_from_slice(&handshake.feed(data)?);
                    if handshake.phase() == Phase::Ready {
                        // 握手完成时缓冲里可能已粘着传输帧：一并解出。
                        incoming.extend(handshake.open(&[])?);
                    }
                } else {
                    incoming.extend(handshake.open(data)?);
                }
            }
        }
        let mut messages = Vec::with_capacity(incoming.len());
        for (message_type, payload) in incoming {
            let (responses, decoded) = self.dispatch(message_type, &payload)?;
            for (response_type, response_payload) in responses {
                out.extend_from_slice(&self.encode(response_type, &response_payload)?);
            }
            messages.push(decoded);
            if self.closed {
                break;
            }
        }
        Ok(FeedOutcome {
            out,
            messages,
            phase: self.phase().to_string(),
            subscribed: self.subscribed,
            closed: self.closed,
        })
    }

    /// 推送状态变化（订阅态下才产出帧）；`updates` 为 `[{key, state}]`。
    ///
    /// 返回需要写回对端的帧字节（未订阅 / 无变化 = 空）。
    pub(crate) fn apply_states(&mut self, updates: &Value) -> Result<Vec<u8>, String> {
        if self.closed {
            return Err("会话已关闭".to_string());
        }
        let updates = validate_updates(&self.entities, updates)?;
        let entities = self.entities.as_array_mut().ok_or("entities 必须是数组")?;
        for update in &updates {
            let entity = entities
                .iter_mut()
                .find(|e| e["key"] == update["key"])
                .ok_or("未知实体 key")?;
            entity["state"] = update["state"].clone();
        }
        if !self.subscribed {
            return Ok(Vec::new());
        }
        let responses = self.state_messages_for(Some(&updates))?;
        let mut out = Vec::new();
        for (message_type, payload) in responses {
            out.extend_from_slice(&self.encode(message_type, &payload)?);
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // 消息派发（设备侧）
    // ------------------------------------------------------------------

    fn dispatch(
        &mut self,
        message_type: u16,
        payload: &[u8],
    ) -> Result<(Vec<(u16, Vec<u8>)>, Value), String> {
        let mut decoded = json!({
            "type": message_type,
            "message": message_name(message_type),
            "payload_hex": hex_encode(payload),
        });
        if let Ok(small) = u8::try_from(message_type) {
            crate::attach_decoded(small, payload, &mut decoded)?;
        }
        let responses: Vec<(u16, Vec<u8>)> = match message_type {
            // HelloRequest → HelloResponse（服务端版本 / server_info / 设备名）。
            1 => vec![(
                2,
                api_catalog::hello_response_payload(
                    API_VERSION_MAJOR,
                    API_VERSION_MINOR,
                    SERVER_INFO,
                    self.identity_str("name"),
                ),
            )],
            // 兼容旧客户端的 AuthenticationRequest（无密码，空成功应答）。
            3 => vec![(4, Vec::new())],
            // DeviceInfoRequest → DeviceInfoResponse（身份面）。
            9 => vec![(10, self.device_info_payload())],
            // PingRequest → PingResponse（空载荷）。
            7 => vec![(8, Vec::new())],
            // ListEntitiesRequest → 逐实体定义 + Done。
            11 => self.entity_list_messages()?,
            // SubscribeStatesRequest → 全量状态快照 + 置订阅态。
            20 => {
                self.subscribed = true;
                self.state_messages()?
            }
            // DisconnectRequest → DisconnectResponse + 关闭。
            5 => {
                self.closed = true;
                vec![(6, Vec::new())]
            }
            // 其余（命令类 / PingResponse 等）只解出不回包：数据面由宿主执行。
            _ => Vec::new(),
        };
        Ok((responses, decoded))
    }

    fn device_info_payload(&self) -> Vec<u8> {
        let text = |key: &str| self.identity_str(key).to_string();
        api_catalog::device_info_response_payload(
            &text("name"),
            &text("mac"),
            &text("esphome_version"),
            &text("model"),
            &text("manufacturer"),
            &text("friendly_name"),
            &text("suggested_area"),
            self.identity
                .get("api_encryption_supported")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            self.identity
                .get("legacy_bluetooth_proxy_version")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32,
            self.identity
                .get("bluetooth_proxy_feature_flags")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32,
        )
    }

    /// 目录消息：按 `kind` 选择实体定义消息类型，末条 `ListEntitiesDoneResponse`（19）。
    fn entity_list_messages(&self) -> Result<Vec<(u16, Vec<u8>)>, String> {
        let mut responses = Vec::new();
        for entity in entities_of(&self.entities) {
            let key = entity_field_u32(entity, "key")?;
            let kind = entity_field_str(entity, "kind")?;
            let object_id = entity_field_str(entity, "object_id")?;
            let name = entity_field_str(entity, "name")?;
            match kind {
                "binary_sensor" => responses.push((
                    12,
                    api_catalog::list_entities_binary_sensor_payload(
                        object_id,
                        key,
                        name,
                        entity
                            .get("device_class")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    ),
                )),
                "number" => responses.push((
                    49,
                    api_catalog::list_entities_number_payload(
                        object_id,
                        key,
                        name,
                        entity_number(entity, "min_value")?,
                        entity_number(entity, "max_value")?,
                        entity_number(entity, "step")?,
                        entity
                            .get("unit_of_measurement")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        entity.get("mode").and_then(Value::as_u64).unwrap_or(0) as u32,
                    ),
                )),
                other => return Err(format!("实体 {key} 种类非法：{other}")),
            }
        }
        responses.push((19, Vec::new())); // ListEntitiesDoneResponse
        Ok(responses)
    }

    /// 状态消息：`updates = None` 时输出全量快照，`Some` 时按 key 匹配。
    fn state_messages(&self) -> Result<Vec<(u16, Vec<u8>)>, String> {
        self.state_messages_for(None)
    }

    fn state_messages_for(&self, updates: Option<&[Value]>) -> Result<Vec<(u16, Vec<u8>)>, String> {
        let mut responses = Vec::new();
        for entity in entities_of(&self.entities) {
            let key = entity_field_u32(entity, "key")?;
            let kind = entity_field_str(entity, "kind")?;
            if updates.is_some_and(|u| !u.iter().any(|v| v["key"] == entity["key"])) {
                continue;
            }
            let state = entity.get("state").ok_or("实体缺少 state")?;
            match (kind, state) {
                ("binary_sensor", Value::Bool(on)) => responses.push((
                    21,
                    api_catalog::binary_sensor_state_payload(key, *on, false, 0),
                )),
                ("number", Value::Number(number)) => responses.push((
                    50,
                    api_catalog::number_state_payload(
                        key,
                        number.as_f64().unwrap_or(0.0) as f32,
                        false,
                        0,
                    ),
                )),
                _ => {
                    return Err(format!(
                        "实体 {key} 状态类型非法（{kind} 需要 {}）",
                        if kind == "binary_sensor" {
                            "bool"
                        } else {
                            "number"
                        }
                    ))
                }
            }
        }
        Ok(responses)
    }

    fn encode(&mut self, message_type: u16, payload: &[u8]) -> Result<Vec<u8>, String> {
        match &mut self.mode {
            Mode::Plaintext => {
                let small = u8::try_from(message_type)
                    .map_err(|_| format!("消息类型越界：{message_type}"))?;
                api_frame::frame_encode(small, payload)
            }
            Mode::Encrypted(handshake) => handshake.seal(message_type, payload),
        }
    }

    // ------------------------------------------------------------------
    // 序列化（插件隔离 kv）
    // ------------------------------------------------------------------

    pub(crate) fn to_json(&self) -> Value {
        let mut value = json!({
            "mode": self.mode_str(),
            "identity": self.identity,
            "entities": self.entities,
            "subscribed": self.subscribed,
            "closed": self.closed,
            "rx_buf": hex_encode(&self.rx_buf),
        });
        if let Mode::Encrypted(handshake) = &self.mode {
            value["handshake"] = handshake.to_json();
        }
        value
    }

    pub(crate) fn from_json(value: &Value) -> Result<Self, String> {
        let mode = value
            .get("mode")
            .and_then(Value::as_str)
            .ok_or("会话状态缺少 mode")?;
        let identity = validate_identity(value.get("identity").ok_or("会话状态缺少 identity")?)?;
        let entities = validate_entities(value.get("entities").ok_or("会话状态缺少 entities")?)?;
        let mode = match mode {
            "plaintext" => Mode::Plaintext,
            "encrypted" => Mode::Encrypted(Handshake::from_json(
                value
                    .get("handshake")
                    .ok_or("加密会话缺少 handshake 状态")?,
            )?),
            other => return Err(format!("会话状态 mode 非法：{other}")),
        };
        let rx_buf = crate::hex::hex_decode(
            value.get("rx_buf").and_then(Value::as_str).unwrap_or(""),
            MAX_RX_BUF,
            "rx_buf",
        )?;
        Ok(Self {
            mode,
            identity,
            entities,
            subscribed: value
                .get("subscribed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            closed: value
                .get("closed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            rx_buf,
        })
    }
}

/// guest 内自检（不触网、不读写 kv）：明文会话的 Hello / 目录 / 状态 / 断开全链。
pub(crate) fn self_check() -> Result<(), String> {
    let identity = validate_identity(&json!({"name": "esphome-selfcheck"}))?;
    let entities = validate_entities(&json!([
        {"key": 0, "kind": "binary_sensor", "object_id": "online", "name": "在线",
         "device_class": "connectivity", "state": false},
        {"key": 1, "kind": "number", "object_id": "volume", "name": "音量",
         "min_value": 0.0, "max_value": 127.0, "step": 1.0, "mode": 1, "state": 30.0},
    ]))?;
    let mut session = Session::new_plaintext(identity, entities);

    let hello = session.feed(&api_frame::frame_encode(
        1,
        &api_frame::hello_request_payload("selftest-client", 1, 10),
    )?)?;
    let parsed = api_frame::frame_decode(&hello.out)?;
    if parsed.message_type != 2 {
        return Err(format!(
            "HelloRequest 未得到 HelloResponse（type={}）",
            parsed.message_type
        ));
    }
    let info = api_frame::hello_response_decode(&parsed.payload)?;
    if info.name != "esphome-selfcheck" || info.server_info != SERVER_INFO {
        return Err("HelloResponse 身份字段不一致".to_string());
    }

    let list = session.feed(&api_frame::frame_encode(11, &[])?)?;
    let mut entries = 0usize;
    let mut rest = list.out.clone();
    while let Some(frame) = next_frame_from_buffer(&mut rest)? {
        let parsed = api_frame::frame_decode(&frame)?;
        entries += 1;
        if parsed.message_type != 12 && parsed.message_type != 49 && parsed.message_type != 19 {
            return Err(format!("目录消息类型异常：{}", parsed.message_type));
        }
    }
    if entries != 3 {
        return Err(format!(
            "目录应答消息数异常：{entries}（期望 3 = 2 实体 + done）"
        ));
    }

    let states = session.feed(&api_frame::frame_encode(20, &[])?)?;
    if !states.subscribed {
        return Err("SubscribeStatesRequest 未置订阅态".to_string());
    }
    let updated = session.apply_states(&json!([{"key": 0, "state": true}]))?;
    if updated.is_empty() {
        return Err("订阅态下状态推送未产出帧".to_string());
    }

    let close = session.feed(&api_frame::frame_encode(5, &[])?)?;
    if !close.closed {
        return Err("DisconnectRequest 未关闭会话".to_string());
    }
    Ok(())
}
