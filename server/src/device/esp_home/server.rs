//! ESPHome Native API 桥：TCP accept + 每连接协议应答 + 状态回推 + 控制面命令分发。
//!
//! ## 为什么不用 `esphomeserver::EspHomeServer`
//!
//! v3.0.0 的 `EspHomeServer::start` 是**空壳**：`ListEntitiesRequest` 里实体列表的循环
//! 被 `// TODO: Handle the different entity types` 注释掉，只回 `ListEntitiesDoneResponse`；
//! 其余消息原样回显，且**从不发状态响应**。用它会「编译通过但什么都暴露不出去」。
//! 这里改用低层 `esphomeapi::EspHomeApi`——它已托管 Hello / Authentication / DeviceInfo /
//! Ping / Disconnect 握手与 noise_psk 加密——并自行应答实体列表 / 订阅 / 状态回推 /
//! 可写实体的命令。协议与加密仍由 crate 托管。
//!
//! ## 连接内状态机
//!
//! 1. `EspHomeApi` 自动答 Hello / Authentication / DeviceInfo / Ping / Disconnect；
//! 2. 本模块答 `ListEntitiesRequest` → 逐条实体定义（`binary_sensor` / `number`）+ Done；
//! 3. 本模块答 `SubscribeStatesRequest` → 全部实体当前状态；
//! 4. 状态变化（`state` 广播）→ 对应实体的状态响应实时回推；
//! 5. `NumberCommandRequest`（HA 拖滑块）→ 经 [`super::actions`] 翻成 EchoMuse 下发。

use anyhow::Result;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use esphome_native_api::esphomeapi::EspHomeApi;
use esphome_native_api::parser::ProtoMessage;
use esphome_native_api::proto::{
    BinarySensorStateResponse, ListEntitiesBinarySensorResponse, ListEntitiesDoneResponse,
    ListEntitiesNumberResponse, NumberStateResponse,
};

use super::config::EspHomeConfig;
use super::{actions, bluetooth, state};

/// BLE 代理是否生效：桥自己的开关 **且** 设备侧 `[echomuse].ble_proxy` 都为真。
///
/// 单一真相源是 `echomuse.ble_proxy`（设备扫描/连接的总闸），`[esp_home].ble_proxy`
/// 只是允许桥把它宣告出去。二者任一为假就不宣告，避免客户端看到必然失败的能力。
fn ble_proxy_enabled(
    cfg: &EspHomeConfig,
    engines: Option<&std::sync::Arc<crate::runtime::Engines>>,
) -> bool {
    cfg.ble_proxy && engines.is_some_and(|e| e.config.echomuse.ble_proxy)
}

/// 启动桥：绑定 `port` 并无限 accept。`main` 在 `[esp_home].enabled` 时调用。
pub async fn serve(
    cfg: EspHomeConfig,
    engines: std::sync::Arc<crate::runtime::Engines>,
) -> Result<()> {
    // BLE 代理要两个开关同时为真才宣告：桥自己的 `[esp_home].ble_proxy` 与设备侧的
    // `[echomuse].ble_proxy`。后者才是设备扫描/连接的总闸，只开前者会造成
    // 「客户端看到代理能力、点进去却没有设备可用」——§7 不做清单里明令不谎报能力。
    let ble_proxy = ble_proxy_enabled(&cfg, Some(&engines));

    // 启动前校验（`build()` 会解码 base64 密钥；非法则直接暴露，不带着坏配置起服务）
    let _ = build_api(&cfg, ble_proxy)?;

    let listener = TcpListener::bind(("0.0.0.0", cfg.port))
        .await
        .map_err(|e| anyhow::anyhow!("绑定 0.0.0.0:{} 失败：{e}", cfg.port))?;
    tracing::info!(
        "[esp_home] ESPHome Native API 已监听 0.0.0.0:{}（{} 个实体 = {} binary_sensor + {} number，加密={}，mDNS={}，bluetooth_proxy={}）",
        cfg.port,
        state::ENTITIES.len(),
        state::BINARY_COUNT,
        state::NUMBER_COUNT,
        if cfg.encryption_key.is_empty() {
            "明文"
        } else {
            "noise_psk"
        },
        cfg.mdns,
        if ble_proxy { "已宣告" } else { "未宣告" }
    );

    // 实体状态全部来自既有真相源（registry 会话态 + echomuse 设备遥测），
    // 桥只做只读观测并翻译，不自己推导——避免出现第二份会漂移的状态。
    spawn_state_probe(engines.registry.clone());

    if cfg.mdns {
        spawn_mdns(&cfg);
    }

    accept_loop(listener, cfg, engines).await
}

/// accept 循环。**单独抽出**是为了让测试能绑 `127.0.0.1:0`（随机端口）跑真实协议往返，
/// 不必经过 `serve` 的固定端口与 mDNS 广播。
async fn accept_loop(
    listener: TcpListener,
    cfg: EspHomeConfig,
    engines: std::sync::Arc<crate::runtime::Engines>,
) -> Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        let cfg = cfg.clone();
        let engines = engines.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, &cfg, Some(engines)).await {
                tracing::debug!("[esp_home] 连接 {peer} 结束：{e:#}");
            }
        });
    }
}

/// 构造 ESPHome API 句柄（server 角色：本端在 accept 到的流上应答对端请求）。
///
/// `build()` 是 fallible 的：唯一失败原因是 `encryption_key` 不是合法 base64，
/// 故把它当作「启动前配置校验」复用。
fn build_api(cfg: &EspHomeConfig, ble_proxy: bool) -> Result<EspHomeApi> {
    let base = EspHomeApi::builder()
        .name(cfg.device_name.clone())
        .friendly_name(cfg.friendly_name.clone())
        .model(cfg.model.clone())
        .manufacturer(cfg.manufacturer.clone())
        .suggested_area(cfg.suggested_area.clone())
        .api_version_major(1)
        .api_version_minor(10)
        // 客户端在 api >= 1.9 时只读 feature_flags（legacy 版本号仅旧客户端兜底）。
        // 能力位必须与 `bluetooth.rs` 实际实现一致，否则客户端会摆出必然失败的操作。
        .legacy_bluetooth_proxy_version(if ble_proxy {
            bluetooth::LEGACY_VERSION
        } else {
            0
        })
        .bluetooth_proxy_feature_flags(if ble_proxy {
            bluetooth::FEATURE_FLAGS
        } else {
            0
        });
    let built = if cfg.encryption_key.is_empty() {
        base.build()
    } else {
        base.encryption_key(cfg.encryption_key.clone()).build()
    };
    built.map_err(|e| anyhow::anyhow!("ESPHome API 配置非法（多为 encryption_key 非合法 base64）：{e}"))
}

/// 单条客户端连接：取发送端/接收端，循环应答协议请求并回推状态变化。
///
/// `bt` 为 `None` 时（协议往返测试）不启动 BLE 代理任务，BLE 相关消息直接丢弃——
/// 测试只验证实体面，不依赖引擎，也不该被 BLE 代理拖进来。
async fn handle_conn(
    stream: TcpStream,
    cfg: &EspHomeConfig,
    bt: Option<std::sync::Arc<crate::runtime::Engines>>,
) -> Result<()> {
    let peer = stream.peer_addr().ok();

    // ⚠️ 必须先等到对端首字节，再交棒给 `EspHomeApi::start`。
    // 该 crate 的 `start()` 是按「客户端主动连服务端」写的：握手第一步就对流做
    // `fill_buf()` 并要求「已有数据」，否则立刻以 `HandshakeError::NoData` 断开。
    // 我们是服务端，accept 之后客户端可能一个字节都还没发，直接 start 会 100% 断连。
    // `peek` 不消费数据（字节留在内核缓冲，随后 `fill_buf` 照常读到），返回 0 = 对端已断。
    let mut probe = [0u8; 1];
    if stream.peek(&mut probe).await? == 0 {
        tracing::debug!("[esp_home] 连接 {peer:?} 建立即断开（未发任何数据）");
        return Ok(());
    }

    let api = build_api(cfg, ble_proxy_enabled(cfg, bt.as_ref()))?;
    let conn = api
        .start(stream)
        .await
        .map_err(|e| anyhow::anyhow!("协议握手失败：{e}"))?;
    tracing::debug!("[esp_home] ESPHome 客户端已连接 {peer:?}");

    let tx = conn.sender();
    let mut rx = conn.receiver();
    let mut changes = state::subscribe();

    // BLE 代理跑在**独立任务**里：一次 `connect` 最长等 35s，若在下面的 select 里
    // await，整个连接期间连状态回推都会停摆。这里只做转发。
    let bt_tx = bt.map(|engines| {
        let (bt_tx, bt_rx) = mpsc::unbounded_channel();
        bluetooth::spawn(engines, bt_rx, tx.clone());
        bt_tx
    });

    loop {
        tokio::select! {
            incoming = rx.recv() => match incoming {
                // BLE 消息先截走（必须排在其它分支之前，否则会被末尾的 `Ok(_)` 吞掉）。
                Ok(msg) if bt_tx.is_some() && bluetooth::is_bluetooth(&msg) => {
                    if let Some(bt) = &bt_tx {
                        if bt.send(msg).is_err() {
                            tracing::debug!("[esp_home] BLE 代理任务已退出，丢弃消息");
                        }
                    }
                }
                Ok(ProtoMessage::ListEntitiesRequest(_)) => {
                    tracing::debug!("[esp_home] ListEntitiesRequest");
                    send_entity_list(&tx).await;
                }
                Ok(ProtoMessage::SubscribeStatesRequest(_)) => {
                    tracing::debug!("[esp_home] SubscribeStatesRequest");
                    send_all_states(&tx).await;
                }
                // HA 拖音量滑块：命令经翻译缝下发到设备，**不回推**（设备会自己上报
                // `volume_state`，探针读真实值再推；提前推会把滑块弹回旧值）。
                Ok(ProtoMessage::NumberCommandRequest(req)) => {
                    match number_command_target(req.key, req.state) {
                        Some(level) => {
                            tracing::debug!("[esp_home] 音量命令 {:.2} → 档位 {level}", req.state);
                            actions::set_volume(level);
                        }
                        None => tracing::debug!("[esp_home] 忽略未知 number 命令 key={}", req.key),
                    }
                }
                // 其余消息（日志订阅、时间请求等）当前忽略。
                Ok(_) => {}
                Err(RecvError::Lagged(n)) => {
                    tracing::debug!("[esp_home] 客户端消息滞后，跳过 {n} 条");
                }
                Err(RecvError::Closed) => break,
            },
            change = changes.recv() => match change {
                Ok(key) => send_current(&tx, key).await,
                // 丢事件就全量补一次，保证客户端最终一致。
                Err(RecvError::Lagged(_)) => send_all_states(&tx).await,
                Err(RecvError::Closed) => {}
            },
        }
    }
    tracing::debug!("[esp_home] ESPHome 客户端断开 {peer:?}");
    Ok(())
}

/// 应答 `ListEntitiesRequest`：逐条实体定义（按种类选消息类型）+ `ListEntitiesDoneResponse`。
async fn send_entity_list(tx: &mpsc::Sender<ProtoMessage>) {
    for (key, spec) in state::ENTITIES.iter().enumerate() {
        let msg = match spec.kind {
            state::EntityKind::Binary { device_class } => {
                ProtoMessage::ListEntitiesBinarySensorResponse(ListEntitiesBinarySensorResponse {
                    object_id: spec.object_id.to_string(),
                    key: key as u32,
                    name: spec.name.to_string(),
                    device_class: device_class.to_string(),
                    ..Default::default()
                })
            }
            state::EntityKind::Number {
                min,
                max,
                step,
                unit,
                mode,
            } => {
                ProtoMessage::ListEntitiesNumberResponse(ListEntitiesNumberResponse {
                    object_id: spec.object_id.to_string(),
                    key: key as u32,
                    name: spec.name.to_string(),
                    min_value: min,
                    max_value: max,
                    step,
                    unit_of_measurement: unit.to_string(),
                    mode,
                    ..Default::default()
                })
            }
        };
        if tx.send(msg).await.is_err() {
            return;
        }
    }
    let _ = tx
        .send(ProtoMessage::ListEntitiesDoneResponse(
            ListEntitiesDoneResponse {},
        ))
        .await;
}

/// 按实体种类回推它的当前状态。
async fn send_current(tx: &mpsc::Sender<ProtoMessage>, key: usize) {
    match state::ENTITIES.get(key).map(|spec| spec.kind) {
        Some(state::EntityKind::Binary { .. }) => {
            let on = state::binary_snapshot().get(key).copied().unwrap_or(false);
            send_binary_state(tx, key as u32, on).await;
        }
        Some(state::EntityKind::Number { .. }) => {
            send_number_state(tx, key as u32, state::number_value(key)).await;
        }
        None => {}
    }
}

/// 发一个 `binary_sensor` 的当前状态。
async fn send_binary_state(tx: &mpsc::Sender<ProtoMessage>, key: u32, on: bool) {
    let resp = BinarySensorStateResponse {
        key,
        state: on,
        missing_state: false,
        device_id: 0,
    };
    let _ = tx
        .send(ProtoMessage::BinarySensorStateResponse(resp))
        .await;
}

/// 发一个 `number` 的当前值。
async fn send_number_state(tx: &mpsc::Sender<ProtoMessage>, key: u32, value: f32) {
    let resp = NumberStateResponse {
        key,
        state: value,
        missing_state: false,
        device_id: 0,
    };
    let _ = tx.send(ProtoMessage::NumberStateResponse(resp)).await;
}

/// 应答 `SubscribeStatesRequest`：全量状态快照（按 [`state::ENTITIES`] 顺序）。
async fn send_all_states(tx: &mpsc::Sender<ProtoMessage>) {
    for key in 0..state::ENTITIES.len() {
        send_current(tx, key).await;
    }
}

/// `number` 命令的分派规则：**只有音量实体接受命令**，其余返回 `None`（忽略）。
///
/// 抽成纯函数便于测试"命令落到哪个动作"——协议解析（`handle_conn`）与业务翻译
/// （[`actions`]）分离。
fn number_command_target(key: u32, state: f32) -> Option<u8> {
    (key as usize == state::VOLUME).then(|| actions::quantize_volume(state))
}

/// 每 2s 用既有真相源校正实体状态。
///
/// - **会话态**（聆听 / 思考 / 播报）来自 `device::registry` 的活会话表（`Phase`）；
/// - **设备态**（在线 / 静音 / 音量）来自 `device::echomuse` 的 hub 登记与设备遥测。
///
/// 桥只做只读观测，不自己维护状态表——避免出现第二份会漂移的真相。首轮 `tick()`
/// 立即返回，故启动后无需等 2s 就能得到正确初值。
///
/// 遥测缺失（设备还没上报过音量/静音）时**保持上次值**，不写 0：否则 HA 里会出现
/// "音量突然归零"这种假变化。
fn spawn_state_probe(registry: crate::device::registry::Registry) {
    use crate::device::registry::Phase;

    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            tick.tick().await;

            let sessions = registry.sessions();
            state::set_listening(sessions.iter().any(|s| s.phase == Phase::Listening));
            state::set_thinking(sessions.iter().any(|s| s.phase == Phase::Thinking));
            state::set_speaking(sessions.iter().any(|s| s.phase == Phase::Speaking));

            state::set_online(actions::online());
            let telemetry = actions::telemetry();
            if let Some(volume) = telemetry.volume {
                state::set_volume(volume);
            }
            if let Some(muted) = telemetry.muted {
                state::set_muted(muted);
            }
        }
    });
}

/// 广播 mDNS `_esphomelib._tcp`（HA 自动发现）。失败只告警：客户端仍可填 IP:port 手动添加。
fn spawn_mdns(cfg: &EspHomeConfig) {
    let device_name = cfg.device_name.clone();
    let friendly_name = cfg.friendly_name.clone();
    let manufacturer = cfg.manufacturer.clone();
    let model = cfg.model.clone();
    let port = cfg.port;
    let encrypted = !cfg.encryption_key.is_empty();
    let mac = fake_mac(&device_name);

    tokio::task::spawn_blocking(move || {
        use mdns_sd::{ServiceDaemon, ServiceInfo};

        let ip = match crate::device::local_ipv4() {
            Some(ip) => ip.to_string(),
            None => {
                tracing::warn!(
                    "[esp_home] 无法探测局域网 IPv4（容器部署常见），mDNS 跳过；\
                     HA 可用 IP:{} 手动添加",
                    port
                );
                return;
            }
        };
        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("[esp_home] mDNS daemon 启动失败（可用 IP:port 手动添加）：{e}");
                return;
            }
        };
        let mut props = std::collections::HashMap::new();
        props.insert("version".to_string(), "2026.6.2".to_string());
        props.insert("mac".to_string(), mac);
        props.insert("platform".to_string(), "ESP32".to_string());
        props.insert("board".to_string(), "esp32dev".to_string());
        props.insert("network".to_string(), "wifi".to_string());
        props.insert("friendly_name".to_string(), friendly_name);
        props.insert("manufacturer".to_string(), manufacturer);
        props.insert("model".to_string(), model);
        if encrypted {
            props.insert(
                "api_encryption".to_string(),
                "Noise_NNpsk0_25519_ChaChaPoly_SHA256".to_string(),
            );
        }
        let host = format!("{device_name}.local.");
        let info = match ServiceInfo::new(
            "_esphomelib._tcp.local.",
            &device_name,
            &host,
            ip,
            port,
            Some(props),
        ) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!("[esp_home] mDNS ServiceInfo 构造失败：{e}");
                return;
            }
        };
        match daemon.register(info) {
            Ok(_) => tracing::info!(
                "[esp_home] mDNS 已广播 _esphomelib._tcp（HA 可自动发现；name={device_name}）"
            ),
            Err(e) => tracing::warn!("[esp_home] mDNS 注册失败（可 IP:port 手动添加）：{e}"),
        }
        // 注册后 daemon 需常驻才有意义：把它的生命周期交给进程（后台线程自持）。
        std::mem::forget(daemon);
    });
}

/// 稳定的伪 MAC（FNV-1a 32 位 + 固定前缀），仅供 mDNS 展示，不参与协议。
fn fake_mac(seed: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in seed.as_bytes() {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x0100_0193);
    }
    let b = h.to_be_bytes();
    format!(
        "02:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        b[0],
        b[1],
        b[2],
        b[3],
        (h >> 8) as u8
    )
}

#[cfg(test)]
mod tests {
    //! 协议往返测试：不依赖 ASR/TTS 引擎、不依赖模型文件，直接验证桥是否真的「说对了话」。
    //!
    //! 用裸 TCP + 手写帧（`0x00` 标记 + 变长载荷长度 + 类型字节 + protobuf 载荷）扮演
    //! ESPHome 客户端，覆盖 `handle_conn` 的四条真实路径：Hello 握手（由 crate 托管）→
    //! DeviceInfo → 实体列表 → 订阅快照 → 状态变化回推。

    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// 消息类型号（取自 crate `parser::proto_message_mappings!`）。
    const HELLO_REQUEST: u8 = 1;
    const HELLO_RESPONSE: u8 = 2;
    const DEVICE_INFO_REQUEST: u8 = 9;
    const DEVICE_INFO_RESPONSE: u8 = 10;
    const LIST_ENTITIES_REQUEST: u8 = 11;
    const LIST_ENTITIES_BINARY_SENSOR: u8 = 12;
    const LIST_ENTITIES_DONE: u8 = 19;
    const SUBSCRIBE_STATES_REQUEST: u8 = 20;
    const BINARY_SENSOR_STATE: u8 = 21;
    const LIST_ENTITIES_NUMBER: u8 = 49;
    const NUMBER_STATE: u8 = 50;
    /// 客户端 → 服务端的「设定数值」命令（HA 拖滑块）。
    const NUMBER_COMMAND: u8 = 51;

    fn encode_varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    /// 明文帧：`0x00` + 载荷长度（不含类型字节）+ 类型 + 载荷。
    fn frame(msg_type: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8];
        encode_varint(payload.len() as u64, &mut out);
        out.push(msg_type);
        out.extend_from_slice(payload);
        out
    }

    /// 读一帧，返回（类型，载荷）。
    async fn read_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
        let mut marker = [0u8; 1];
        stream.read_exact(&mut marker).await.unwrap();
        assert_eq!(marker[0], 0, "本端应回明文帧（未配 encryption_key）");

        let mut len: u64 = 0;
        let mut shift = 0;
        loop {
            let mut b = [0u8; 1];
            stream.read_exact(&mut b).await.unwrap();
            len |= u64::from(b[0] & 0x7f) << shift;
            if b[0] & 0x80 == 0 {
                break;
            }
            shift += 7;
        }

        // 长度不含类型字节，故多读 1 字节取类型。
        let mut buf = vec![0u8; len as usize + 1];
        stream.read_exact(&mut buf).await.unwrap();
        (buf[0], buf[1..].to_vec())
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    /// 读一个 protobuf varint，返回（值，占用的字节数）。
    fn read_varint(buf: &[u8]) -> (u64, usize) {
        let mut value = 0u64;
        let mut shift = 0;
        for (i, b) in buf.iter().enumerate() {
            value |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return (value, i + 1);
            }
            shift += 7;
        }
        panic!("varint 未结束");
    }

    /// 取某个数值字段；**字段缺省返回 0**——protobuf 会省略默认值，`key=0` / `state=false`
    /// 在线上就是"不出现"，不能靠"载荷前两字节"判断。
    ///
    /// 同时支持 varint（`state` 等 bool/uint32）与 fixed32（`key`：ESPHome 的
    /// `BinarySensorStateResponse.key` 在当前 proto 版本是 `fixed32`，线上 4 字节小端）。
    fn field_u32(payload: &[u8], want_tag: u8) -> u64 {
        let mut i = 0;
        while i < payload.len() {
            let (key, n) = read_varint(&payload[i..]);
            i += n;
            let tag = (key >> 3) as u8;
            match key & 7 {
                // varint
                0 => {
                    let (value, n) = read_varint(&payload[i..]);
                    i += n;
                    if tag == want_tag {
                        return value;
                    }
                }
                // fixed32（小端 4 字节）
                5 => {
                    let bytes: [u8; 4] = payload[i..i + 4].try_into().unwrap();
                    i += 4;
                    if tag == want_tag {
                        return u64::from(u32::from_le_bytes(bytes));
                    }
                }
                // length-delimited：跳过（字符串/嵌套消息），本测试只查数值字段
                2 => {
                    let (len, n) = read_varint(&payload[i..]);
                    i += n + len as usize;
                }
                // fixed64（double/fixed64）：跳过 8 字节
                1 => i += 8,
                other => panic!("未预期的 wire type {other}（tag={tag}）载荷={payload:02X?}"),
            }
        }
        0
    }

    /// 取某个 `float` 字段（wire type 5，小端 4 字节）；**缺省返回 0.0**——同样因为
    /// protobuf 省略默认值，`min_value=0` / `state=0.0` 在线上不出现。
    fn field_f32(payload: &[u8], want_tag: u8) -> f32 {
        let mut i = 0;
        while i < payload.len() {
            let (key, n) = read_varint(&payload[i..]);
            i += n;
            let tag = (key >> 3) as u8;
            match key & 7 {
                0 => {
                    let (_, n) = read_varint(&payload[i..]);
                    i += n;
                }
                5 => {
                    let bytes: [u8; 4] = payload[i..i + 4].try_into().unwrap();
                    i += 4;
                    if tag == want_tag {
                        return f32::from_le_bytes(bytes);
                    }
                }
                2 => {
                    let (len, n) = read_varint(&payload[i..]);
                    i += n + len as usize;
                }
                1 => i += 8,
                other => panic!("未预期的 wire type {other}（tag={tag}）载荷={payload:02X?}"),
            }
        }
        0.0
    }

    /// 一次完整往返：握手 → 设备信息 → 实体定义（5 binary + 1 number）→ 订阅快照 →
    /// 二值/数值状态实时回推 → `number` 控制命令（连接保持可用）。
    ///
    /// 连接处理**内联**跑（不经 accept_loop 的 spawn），这样 `handle_conn` 的失败原因
    /// 会直接以 `Err` 冒出来，而不是变成客户端侧的 `UnexpectedEof` 哑谜。
    #[tokio::test]
    async fn plaintext_roundtrip_and_state_push() {
        // `esphome-native-api` 用 `log` 打协议日志（读取循环的终止原因），主程序用 tracing。
        // 这里装一个只在测试期存在的 logger，否则 bridge 内部报错完全静默。
        let _ = env_logger::builder().is_test(true).try_init();

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cfg = EspHomeConfig {
            port: addr.port(),
            mdns: false,
            ..Default::default()
        };

        // 先并发建连 + accept，再把客户端的探测逻辑丢到独立任务里。
        let (connect, accept) = tokio::join!(TcpStream::connect(addr), listener.accept());
        let mut stream = connect.unwrap();
        let (server_stream, _) = accept.unwrap();

        let client = tokio::spawn(async move {
            // 1) Hello：字段号是 client_info=1 / major=2 / minor=3（与 api.proto 一致），
            //    应答由 crate 托管。proto 版本 1.10 与本桥上报的一致。
            let mut hello = vec![0x0a, 0x04];
            hello.extend_from_slice(b"test");
            hello.extend_from_slice(&[0x10, 0x01, 0x18, 0x0a]);
            stream.write_all(&frame(HELLO_REQUEST, &hello)).await.unwrap();
            let (ty, _) = read_frame(&mut stream).await;
            assert_eq!(ty, HELLO_RESPONSE, "期望 HelloResponse");

        // 2) DeviceInfo：应带上配置里的节点名。
            stream
                .write_all(&frame(DEVICE_INFO_REQUEST, &[]))
                .await
                .unwrap();
            let (ty, payload) = read_frame(&mut stream).await;
            assert_eq!(ty, DEVICE_INFO_RESPONSE, "期望 DeviceInfoResponse");
            assert!(
                contains(&payload, b"xiaozhi-dot2"),
                "DeviceInfo 应含节点名"
            );

            // 3) 实体列表：5 条 binary_sensor + 1 条 number + Done，object_id 按声明顺序。
            stream
                .write_all(&frame(LIST_ENTITIES_REQUEST, &[]))
                .await
                .unwrap();
            let mut listed: Vec<(u8, Vec<u8>)> = Vec::new();
            loop {
                let (ty, payload) = read_frame(&mut stream).await;
                if ty == LIST_ENTITIES_DONE {
                    break;
                }
                listed.push((ty, payload));
            }
            let expected: [(u8, &str); 6] = [
                (LIST_ENTITIES_BINARY_SENSOR, "dot2_online"),
                (LIST_ENTITIES_BINARY_SENSOR, "dot2_listening"),
                (LIST_ENTITIES_BINARY_SENSOR, "dot2_thinking"),
                (LIST_ENTITIES_BINARY_SENSOR, "dot2_speaking"),
                (LIST_ENTITIES_BINARY_SENSOR, "dot2_muted"),
                (LIST_ENTITIES_NUMBER, "dot2_volume"),
            ];
            assert_eq!(listed.len(), expected.len(), "实体数量应与 ENTITIES 一致");
            for (i, (want_ty, object_id)) in expected.iter().enumerate() {
                assert_eq!(listed[i].0, *want_ty, "第 {i} 个实体的类型不符");
                assert!(
                    contains(&listed[i].1, object_id.as_bytes()),
                    "第 {i} 个实体应含 object_id={object_id}"
                );
            }
            // 音量的类型参数必须是 0..127 / 步长 1，否则 HA 的滑块范围是错的。
            let volume = &listed[5].1;
            assert_eq!(field_f32(volume, 6), 0.0, "min_value=0 属默认值，线上省略");
            assert_eq!(field_f32(volume, 7), 127.0, "max_value 应为 127");
            assert_eq!(field_f32(volume, 8), 1.0, "step 应为 1");

            // 4) 订阅：全量快照 = 5 条 binary + 1 条 number，key 与实体索引一致。
            stream
                .write_all(&frame(SUBSCRIBE_STATES_REQUEST, &[]))
                .await
                .unwrap();
            for i in 0..state::BINARY_COUNT as u64 {
                let (ty, payload) = read_frame(&mut stream).await;
                assert_eq!(ty, BINARY_SENSOR_STATE, "期望 BinarySensorStateResponse");
                assert_eq!(
                    field_u32(&payload, 1),
                    i,
                    "key 应为实体索引 {i}（缺省即 0）"
                );
                assert_eq!(field_u32(&payload, 2), 0, "初始状态应为 false");
            }
            let (ty, payload) = read_frame(&mut stream).await;
            assert_eq!(ty, NUMBER_STATE, "期望 NumberStateResponse");
            assert_eq!(
                field_u32(&payload, 1),
                state::VOLUME as u64,
                "音量 key 应为 VOLUME"
            );
            assert_eq!(field_f32(&payload, 2), 0.0, "初始音量应为 0");

            // 5) 二值状态回推：speaking(key=3) 置 true 应立即推给已订阅的客户端。
            state::set_speaking(true);
            let (ty, payload) = read_frame(&mut stream).await;
            assert_eq!(ty, BINARY_SENSOR_STATE, "期望状态回推");
            assert_eq!(field_u32(&payload, 1), 3, "speaking 的 key 应为 3");
            assert_eq!(field_u32(&payload, 2), 1, "speaking 应被推为 true");
            // 复位也会回推，先吃掉，避免干扰后续断言。
            state::set_speaking(false);
            let _ = read_frame(&mut stream).await;

            // 6) 数值状态回推：音量置 42.5 应以 NumberStateResponse 推给客户端。
            state::set_volume(42.5);
            let (ty, payload) = read_frame(&mut stream).await;
            assert_eq!(ty, NUMBER_STATE, "期望音量回推");
            assert_eq!(
                field_u32(&payload, 1),
                state::VOLUME as u64,
                "音量 key 应为 VOLUME"
            );
            assert_eq!(field_f32(&payload, 2), 42.5, "音量应被推为 42.5");
            state::set_volume(0.0);
            let _ = read_frame(&mut stream).await;

            // 7) 控制命令：发 NumberCommandRequest（key=VOLUME, state=30.0）应被接受且
            //    不中断连接。测试里无设备在线，故下发失败是预期的——这里只验证协议路径
            //    通畅（命令能被解析、连接继续正常应答）。
            let mut cmd = vec![
                0x0d, // key: tag=1, wire type 5 (fixed32)
                state::VOLUME as u8,
                0x00,
                0x00,
                0x00,
                0x15, // state: tag=2, wire type 5 (float)
            ];
            cmd.extend_from_slice(&30.0f32.to_le_bytes());
            stream
                .write_all(&frame(NUMBER_COMMAND, &cmd))
                .await
                .unwrap();
            // 命令不产生回包；用一次 ListEntities 确认连接仍活着。
            stream
                .write_all(&frame(LIST_ENTITIES_REQUEST, &[]))
                .await
                .unwrap();
            let mut count = 0;
            loop {
                let (ty, _) = read_frame(&mut stream).await;
                if ty == LIST_ENTITIES_DONE {
                    break;
                }
                count += 1;
            }
            assert_eq!(count, expected.len(), "命令之后连接仍应正常应答");
        });

        // 内联跑连接处理：客户端探测若失败或提前断开，这里立刻返回 Err（含真实原因），
        // 而不是让客户端卡在 `UnexpectedEof` 上看不出所以然。
        let served = handle_conn(server_stream, &cfg, None).await;
        client.await.expect("客户端探测任务不应 panic");
        served.expect("连接处理应正常结束");
    }

    /// 命令分派：只有音量实体接受命令，其余 key 一律忽略（不误触发别的动作）。
    #[test]
    fn number_command_only_targets_volume() {
        assert_eq!(number_command_target(state::VOLUME as u32, 30.4), Some(30));
        assert_eq!(number_command_target(state::VOLUME as u32, 127.9), Some(127));
        // 其它实体的 key（binary_sensor）与时越界 key 都不应触发动作。
        assert_eq!(number_command_target(state::ONLINE as u32, 50.0), None);
        assert_eq!(number_command_target(999, 50.0), None);
    }
}
