//! ESPHome `bluetooth_proxy` 桥：把 Dot 2 的 BLE 无线电暴露给 native-api 客户端。
//!
//! ## 能力边界（实测自 EchoMuse 设备侧，**勿乐观假设**）
//!
//! 设备控制帧（`device/internal/bluetooth/bridge.go`，见 `echomuse/ble_request.rs`）
//! 只接受 `connect` / `disconnect` / `services` / `read` / `write` / `slots`；
//! `notify` 与 `disconnected` 是设备**自动**推送的事件，没有"按 handle 订阅"的动作；
//! 并且 `echomuse/ble.rs` 首行明确"No pairing/encryption"。
//!
//! 因此本桥能做的与不能做的必须分清：
//!
//! | ESPHome 能力 | 本桥 | 说明 |
//! |---|---|---|
//! | 被动扫描 | ✅ | 设备自己扫，桥轮询快照去重后上推 |
//! | 连接 / 断连 | ✅ | `t:"connect"` / `t:"disconnect"` |
//! | 服务发现 / 读 / 写 | ✅ | `t:"services"` / `read` / `write` |
//! | 连接槽查询 | ✅ | `t:"slots"` |
//! | notify 通知 | ⚠️ | 只能转发设备已自动推送的事件流，无法按 handle 开关 |
//! | 配对 / 解配对 / 清缓存 | ❌ | 设备无此能力，回错误而不是假装成功 |
//! | 描述符读写 | ❌ | 设备无此能力，回错误 |
//!
//! 同理宣告的 `bluetooth_proxy_feature_flags` **只报真实具备的位**
//! （[`FEATURE_FLAGS`]）：谎报会让客户端把必然失败的操作摆到用户面前。
//!
//! ## 广播的语义降级
//!
//! 设备给的是**按 addr 合并后的最新快照**（`adverts: HashMap<addr, entry>`），不是
//! 逐包事件流。故桥按 `(addr, data)` 去重：内容不变就不重复上推，避免设备周期
//! 重报同一批广播时把客户端刷爆。代价是 RSSI 单独变化不会触发上报。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use esphome_native_api::parser::ProtoMessage;
use esphome_native_api::proto::{
    BluetoothConnectionsFreeResponse, BluetoothDeviceConnectionResponse, BluetoothDeviceRequest,
    BluetoothGattCharacteristic, BluetoothGattDescriptor, BluetoothGattErrorResponse,
    BluetoothGattGetServicesDoneResponse, BluetoothGattGetServicesResponse,
    BluetoothGattNotifyDataResponse, BluetoothGattNotifyResponse, BluetoothGattReadResponse,
    BluetoothGattService, BluetoothGattWriteResponse, BluetoothLeAdvertisementResponse,
    BluetoothServiceData,
};

use crate::device::echomuse::ble::{self, BleError, BleSnapshot, GATT_WAIT};
use crate::runtime::Engines;

/// 宣告给客户端的能力位：`PASSIVE_SCAN | ACTIVE_CONNECTIONS`。
///
/// 对应 aioesphomeapi `BluetoothProxyFeature` 的 `1 << 0` 与 `1 << 1`。
/// **不得**加上 `PAIRING`(8) / `CACHE_CLEARING`(16) / `REMOTE_CACHING`(4)——
/// 设备侧没有这些能力，谎报只会换来必然失败的点击。
pub const FEATURE_FLAGS: u32 = 0b011;

/// 旧客户端（api < 1.9）才读的兼容版本号，保持与 `FEATURE_FLAGS` 语义一致。
pub const LEGACY_VERSION: u32 = 3;

/// ESPHome 的 GATT 错误码（`esphome/bluetooth_proxy.h` 的 `BluetoothProxyError`）。
const ERR_FAILED: i32 = -1;
const ERR_TIMEOUT: i32 = -2;
const ERR_BUSY: i32 = -4;
const ERR_NOT_FOUND: i32 = -5;
const ERR_NOT_CONNECTED: i32 = -6;

/// 广播轮询周期。设备侧广播缓存不需要更快；500ms 对 HA 的被动扫描足够。
const POLL: Duration = Duration::from_millis(500);

/// 去重表上限（远大于设备侧 256 条广播缓存，超了整表清空重来）。
const SEEN_CAP: usize = 512;

/// MAC 字符串 → ESPHome 的 `address`（6 字节大端放进 u64）。
///
/// `aa:bb:cc:dd:ee:ff` → `0x0000_aabb_ccdd_eeff`。非法格式返回 `None`
/// （调用方据此回 `ERR_FAILED`，不要拿 0 地址去连设备）。
pub fn mac_to_u64(mac: &str) -> Option<u64> {
    let mut addr = 0u64;
    let mut parts = 0;
    for part in mac.split(':') {
        if part.len() != 2 {
            return None;
        }
        let byte = u8::from_str_radix(part, 16).ok()?;
        addr = (addr << 8) | u64::from(byte);
        parts += 1;
    }
    (parts == 6).then_some(addr)
}

/// [`mac_to_u64`] 的逆运算，输出小写 `aa:bb:cc:dd:ee:ff`（设备控制帧要求的形式）。
pub fn u64_to_mac(addr: u64) -> String {
    let b = addr.to_be_bytes();
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        b[2], b[3], b[4], b[5], b[6], b[7]
    )
}

/// UUID 字符串 → 16 字节。接受带/不带连字符的写法。
fn uuid_bytes(uuid: &str) -> Option<Vec<u8>> {
    let hex: String = uuid.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    (0..16)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// ESPHome 的 `uuid` 字段：128-bit UUID 拆成两个 u64（各按大端）。
///
/// 客户端按 `b"".join(x.to_bytes(8, "big"))` 还原，故不能按小端拆。
pub fn uuid_words(uuid: &str) -> Vec<u64> {
    let Some(bytes) = uuid_bytes(uuid) else {
        return Vec::new();
    };
    bytes
        .chunks(8)
        .map(|c| u64::from_be_bytes(c.try_into().expect("8 字节一组")))
        .collect()
}

/// 仅当是蓝牙基数 UUID（`0000xxxx-0000-1000-8000-00805f9b34fb`）时返回 16-bit 值。
///
/// 其它 UUID 返回 0 = "没有短 UUID"，客户端此时改用 `uuid` 字段。
pub fn short_uuid(uuid: &str) -> u32 {
    const BASE: &str = "00001000800000805f9b34fb";
    let Some(bytes) = uuid_bytes(uuid) else {
        return 0;
    };
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if hex[8..] != *BASE {
        return 0;
    }
    u32::from_str_radix(&hex[4..8], 16).unwrap_or(0)
}

/// 解析后的广播数据（BLE AD structures）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AdData {
    /// `0x08` / `0x09`：设备名（UTF-8 有损解码）。
    pub local_name: Option<String>,
    /// `0x02`~`0x07`：服务 UUID（展开为完整 128-bit 字符串）。
    pub service_uuids: Vec<String>,
    /// `0x16` / `0x20` / `0x21`：服务数据（完整 UUID + 负载）。
    pub service_data: Vec<(String, Vec<u8>)>,
    /// `0xFF`：厂商数据（company id + 负载）。**小米 MiBeacon 就在这一项**（`0xFE95`）。
    pub manufacturer_data: Vec<(u16, Vec<u8>)>,
}

/// 解析 BLE 广播的 AD 结构。
///
/// 格式：`[长度][类型][值...]`，长度为 0 表示结束。**对截断/畸形输入必须容错**：
/// 广播来自空口，任何长度都不会越界 panic，解析到第一个不合法处即停。
pub fn parse_ad(raw: &[u8]) -> AdData {
    let mut out = AdData::default();
    let mut i = 0usize;
    while i < raw.len() {
        let len = raw[i] as usize;
        i += 1;
        if len == 0 {
            break;
        }
        // len 覆盖「类型 1 字节 + 值 len-1 字节」
        if len < 1 || i + len > raw.len() {
            break;
        }
        let ty = raw[i];
        let value = &raw[i + 1..i + len];
        i += len;
        match ty {
            0x02 | 0x03 => push_uuids16(&mut out.service_uuids, value),
            0x04 | 0x05 => push_uuids32(&mut out.service_uuids, value),
            0x06 | 0x07 => push_uuids128(&mut out.service_uuids, value),
            0x08 | 0x09 => {
                out.local_name = Some(String::from_utf8_lossy(value).into_owned());
            }
            0x16 => {
                if value.len() >= 2 {
                    let uuid = uuid16(u16::from_le_bytes([value[0], value[1]]));
                    out.service_data.push((uuid, value[2..].to_vec()));
                }
            }
            0x20 => {
                if value.len() >= 4 {
                    let uuid = uuid32(u32::from_le_bytes([
                        value[0], value[1], value[2], value[3],
                    ]));
                    out.service_data.push((uuid, value[4..].to_vec()));
                }
            }
            0x21 => {
                if value.len() >= 16 {
                    let uuid = uuid128(&value[..16]);
                    out.service_data.push((uuid, value[16..].to_vec()));
                }
            }
            0xFF => {
                if value.len() >= 2 {
                    out.manufacturer_data
                        .push((u16::from_le_bytes([value[0], value[1]]), value[2..].to_vec()));
                }
            }
            // flags(0x01) / TX power(0x0A) / 外观(0x19) 等对代理无用，忽略。
            _ => {}
        }
    }
    out
}

fn push_uuids16(out: &mut Vec<String>, value: &[u8]) {
    for c in value.chunks_exact(2) {
        out.push(uuid16(u16::from_le_bytes([c[0], c[1]])));
    }
}

fn push_uuids32(out: &mut Vec<String>, value: &[u8]) {
    for c in value.chunks_exact(4) {
        out.push(uuid32(u32::from_le_bytes([c[0], c[1], c[2], c[3]])));
    }
}

fn push_uuids128(out: &mut Vec<String>, value: &[u8]) {
    for c in value.chunks_exact(16) {
        out.push(uuid128(c));
    }
}

/// 16-bit UUID → 蓝牙基数 UUID 字符串（小写，与 esphome 的 `to_string()` 一致）。
fn uuid16(v: u16) -> String {
    format!("0000{v:04x}-0000-1000-8000-00805f9b34fb")
}

fn uuid32(v: u32) -> String {
    format!("{v:08x}-0000-1000-8000-00805f9b34fb")
}

/// 128-bit UUID 在空口按小端排列，输出时翻转成大端书写顺序。
fn uuid128(c: &[u8]) -> String {
    let mut b = [0u8; 16];
    b.copy_from_slice(&c[..16]);
    b.reverse();
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// 设备侧 advert JSON → ESPHome 广播响应。解析失败（缺 addr / 非法 addr）返回 `None`。
fn to_advert(entry: &Value) -> Option<BluetoothLeAdvertisementResponse> {
    let addr = mac_to_u64(entry.get("addr")?.as_str()?)?;
    let raw = entry
        .get("data")
        .and_then(Value::as_str)
        .and_then(|s| base64_decode(s))
        .unwrap_or_default();
    let ad = parse_ad(&raw);
    Some(BluetoothLeAdvertisementResponse {
        address: addr,
        // ESPHome 的 name 是 bytes（不保证 UTF-8），直接给原始字节最保真。
        name: ad.local_name.map_or_else(Vec::new, String::into_bytes),
        rssi: entry.get("rssi").and_then(Value::as_i64).unwrap_or(0) as i32,
        service_uuids: ad.service_uuids,
        service_data: ad
            .service_data
            .into_iter()
            .map(|(uuid, data)| BluetoothServiceData {
                uuid,
                data,
                // `legacy_data` 已弃用；用 Default 而不是显式赋值，避免 deprecation 警告。
                ..Default::default()
            })
            .collect(),
        manufacturer_data: ad
            .manufacturer_data
            .into_iter()
            .map(|(company, data)| BluetoothServiceData {
                // esphome 侧是 4 位大写 hex 的 company id，不是完整 UUID。
                uuid: format!("{company:04X}"),
                data,
                ..Default::default()
            })
            .collect(),
        address_type: entry.get("addrType").and_then(Value::as_u64).unwrap_or(0) as u32,
    })
}

fn base64_decode(raw: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(raw).ok()
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// 设备侧 `services` 结果 → ESPHome 服务列表。
///
/// ⚠️ 特征值的 `handle` 必须取 **`value_handle`**：设备的 `handle` 是声明句柄，
/// 读写走的是值句柄。取错会让后续 read/write 全部打到错误属性上。
fn to_services(result: &Value) -> Vec<BluetoothGattService> {
    let Some(list) = result.get("services").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|s| {
            let uuid = s.get("uuid")?.as_str()?;
            let characteristics = s
                .get("chars")
                .and_then(Value::as_array)
                .map(|chars| chars.iter().filter_map(to_characteristic).collect())
                .unwrap_or_default();
            Some(BluetoothGattService {
                uuid: uuid_words(uuid),
                // 服务的 handle 用声明起点：客户端只把它当标识，读写不经过它。
                handle: s.get("start").and_then(Value::as_u64).unwrap_or(0) as u32,
                characteristics,
                short_uuid: short_uuid(uuid),
            })
        })
        .collect()
}

fn to_characteristic(c: &Value) -> Option<BluetoothGattCharacteristic> {
    let uuid = c.get("uuid")?.as_str()?;
    let descriptors = c
        .get("descs")
        .and_then(Value::as_array)
        .map(|descs| {
            descs
                .iter()
                .filter_map(|d| {
                    let uuid = d.get("uuid")?.as_str()?;
                    Some(BluetoothGattDescriptor {
                        uuid: uuid_words(uuid),
                        handle: d.get("handle").and_then(Value::as_u64).unwrap_or(0) as u32,
                        short_uuid: short_uuid(uuid),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(BluetoothGattCharacteristic {
        uuid: uuid_words(uuid),
        handle: c.get("value_handle").and_then(Value::as_u64).unwrap_or(0) as u32,
        properties: c.get("props").and_then(Value::as_u64).unwrap_or(0) as u32,
        descriptors,
        short_uuid: short_uuid(uuid),
    })
}

/// `BleError` → ESPHome 的负数错误码。
fn error_code(err: &BleError) -> i32 {
    match err {
        BleError::Timeout => ERR_TIMEOUT,
        BleError::Busy => ERR_BUSY,
        BleError::NotFound => ERR_NOT_FOUND,
        BleError::Offline | BleError::DataOffline | BleError::SessionChanged => ERR_NOT_CONNECTED,
        _ => ERR_FAILED,
    }
}

/// 单连接的 BLE 代理会话。
///
/// 每个 ESPHome 连接一个，独立维护"是否订阅广播""订阅了哪些 notify handle"。
/// 跑在独立任务里（见 [`spawn`]），因此慢的 GATT 操作不会阻塞主循环的状态回推——
/// 这一点很关键：一次 `connect` 最长要等 35s。
struct Proxy {
    /// 被代理的设备 id（Dot 2）。首次需要时从 hub 取，之后复用。
    device: Option<String>,
    /// 客户端是否订阅了广播。
    advert_on: bool,
    /// 客户端要求转发的 notify `(address, handle)`。
    ///
    /// 注意这是**桥侧的转发过滤器**，不是设备侧订阅——设备无条件推送，
    /// 桥只把客户端要的那几路挑出去。
    notify: HashSet<(u64, u32)>,
    /// 广播去重表：`addr → (rssi, data)`。
    seen: HashMap<u64, (i32, String)>,
    /// 上一次已处理的事件（用于在环形事件窗口里定位新事件的起点）。
    last_event: Option<Value>,
}

impl Proxy {
    fn new() -> Self {
        Self {
            device: None,
            advert_on: false,
            notify: HashSet::new(),
            seen: HashMap::new(),
            last_event: None,
        }
    }

    /// 当前被代理的设备 id。Dot 2 是唯一设备，取 hub 里第一台。
    fn device_id(&mut self) -> Option<String> {
        if self.device.is_none() {
            self.device = super::actions::active_device();
        }
        self.device.clone()
    }

    /// 处理一条 bluetooth 消息。返回 `false` 表示连接已断、会话该结束。
    async fn handle(&mut self, engines: &Engines, msg: ProtoMessage, tx: &Tx) -> bool {
        match msg {
            ProtoMessage::SubscribeBluetoothLeAdvertisementsRequest(_) => {
                self.advert_on = true;
                tracing::debug!("[esp_home] 客户端订阅 BLE 广播");
            }
            ProtoMessage::UnsubscribeBluetoothLeAdvertisementsRequest(_) => {
                self.advert_on = false;
                // 重订阅时重新推一遍全量，故清空去重表。
                self.seen.clear();
            }
            ProtoMessage::BluetoothDeviceRequest(req) => {
                self.device_request(engines, req, tx).await;
            }
            ProtoMessage::BluetoothGattGetServicesRequest(req) => {
                let addr = req.address;
                let Some(id) = self.device_id() else {
                    return send(
                        tx,
                        ProtoMessage::BluetoothGattErrorResponse(BluetoothGattErrorResponse {
                            address: addr,
                            handle: 0,
                            error: ERR_NOT_CONNECTED,
                        }),
                    )
                    .await;
                };
                let body = json!({"t":"services","addr":u64_to_mac(addr)});
                match ble::gatt_call(&engines.config, &id, body, GATT_WAIT).await {
                    Ok(v) => {
                        let services = to_services(&v);
                        let ok = send(
                            tx,
                            ProtoMessage::BluetoothGattGetServicesResponse(
                                BluetoothGattGetServicesResponse { address: addr, services },
                            ),
                        )
                        .await
                            && send(
                                tx,
                                ProtoMessage::BluetoothGattGetServicesDoneResponse(
                                    BluetoothGattGetServicesDoneResponse { address: addr },
                                ),
                            )
                            .await;
                        return ok;
                    }
                    Err(e) => return self.gatt_error(tx, addr, 0, &e).await,
                }
            }
            ProtoMessage::BluetoothGattReadRequest(req) => {
                let (addr, handle) = (req.address, req.handle);
                let Some(id) = self.device_id() else {
                    return self.gatt_error(tx, addr, handle, &BleError::Offline).await;
                };
                let body = json!({"t":"read","addr":u64_to_mac(addr),"handle":handle});
                match ble::gatt_call(&engines.config, &id, body, GATT_WAIT).await {
                    Ok(v) => {
                        let data = v
                            .get("value")
                            .and_then(Value::as_str)
                            .and_then(base64_decode)
                            .unwrap_or_default();
                        return send(
                            tx,
                            ProtoMessage::BluetoothGattReadResponse(BluetoothGattReadResponse {
                                address: addr,
                                handle,
                                data,
                            }),
                        )
                        .await;
                    }
                    Err(e) => return self.gatt_error(tx, addr, handle, &e).await,
                }
            }
            ProtoMessage::BluetoothGattWriteRequest(req) => {
                let (addr, handle) = (req.address, req.handle);
                let Some(id) = self.device_id() else {
                    return self.gatt_error(tx, addr, handle, &BleError::Offline).await;
                };
                let body = json!({
                    "t":"write",
                    "addr":u64_to_mac(addr),
                    "handle":handle,
                    "value":base64_encode(&req.data),
                    "response":req.response,
                });
                match ble::gatt_call(&engines.config, &id, body, GATT_WAIT).await {
                    Ok(_) => {
                        return send(
                            tx,
                            ProtoMessage::BluetoothGattWriteResponse(BluetoothGattWriteResponse {
                                address: addr,
                                handle,
                            }),
                        )
                        .await;
                    }
                    Err(e) => return self.gatt_error(tx, addr, handle, &e).await,
                }
            }
            // 设备没有"按 handle 订阅"的动作，只能转发它已自动推送的事件流；
            // 这里记下客户端要哪几路，实际转发在 `poll` 里做。
            ProtoMessage::BluetoothGattNotifyRequest(req) => {
                let key = (req.address, req.handle);
                if req.enable {
                    self.notify.insert(key);
                } else {
                    self.notify.remove(&key);
                }
                return send(
                    tx,
                    ProtoMessage::BluetoothGattNotifyResponse(BluetoothGattNotifyResponse {
                        address: req.address,
                        handle: req.handle,
                    }),
                )
                .await;
            }
            ProtoMessage::SubscribeBluetoothConnectionsFreeRequest(_) => {
                // 连接槽是设备**主动上报**的缓存（`ble.rs` 的 `slots` 字段），不是查询接口：
                // 主动查要排进设备的串行请求队列（最长 35s），对"订阅"语义太慢。
                // 故直接读缓存；设备还没上报过就回 0/0，让客户端自行退避。
                let slots = self
                    .device_id()
                    .and_then(|id| ble::read_snapshot(&engines.config, &id).ok())
                    .and_then(|s| s.slots);
                let num = |key: &str| -> u32 {
                    slots
                        .as_ref()
                        .and_then(|s| s.get(key))
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32
                };
                let allocated = slots
                    .as_ref()
                    .and_then(|s| s.get("addrs"))
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().and_then(mac_to_u64))
                            .collect()
                    })
                    .unwrap_or_default();
                return send(
                    tx,
                    ProtoMessage::BluetoothConnectionsFreeResponse(BluetoothConnectionsFreeResponse {
                        free: num("free"),
                        limit: num("limit"),
                        allocated,
                    }),
                )
                .await;
            }
            // 设备不支持描述符读写：明确回错误，而不是假装成功让客户端挂等。
            ProtoMessage::BluetoothGattReadDescriptorRequest(req) => {
                return self
                    .gatt_error(tx, req.address, req.handle, &BleError::Unsupported)
                    .await;
            }
            ProtoMessage::BluetoothGattWriteDescriptorRequest(req) => {
                return self
                    .gatt_error(tx, req.address, req.handle, &BleError::Unsupported)
                    .await;
            }
            _ => {}
        }
        true
    }

    async fn device_request(&mut self, engines: &Engines, req: BluetoothDeviceRequest, tx: &Tx) -> bool {
        let addr = req.address;
        // ESPHome: 0=Connect(已废弃) 1=Disconnect 2=Pair 3=Unpair
        //          4=ConnectV3WithCache 5=ConnectV3WithoutCache 6=ClearCache
        let op = match req.request_type {
            0 | 4 | 5 => "connect",
            1 => "disconnect",
            2 | 3 | 6 => {
                // 设备侧无配对 / 清缓存能力（`ble.rs`：No pairing/encryption）。
                tracing::debug!(
                    "[esp_home] 拒绝 BLE request_type={} （设备不支持配对/清缓存）",
                    req.request_type
                );
                return send(
                    tx,
                    ProtoMessage::BluetoothDeviceConnectionResponse(
                        BluetoothDeviceConnectionResponse {
                            address: addr,
                            connected: false,
                            mtu: 0,
                            error: ERR_FAILED,
                        },
                    ),
                )
                .await;
            }
            other => {
                tracing::debug!("[esp_home] 未知 BLE request_type={other}");
                return send(
                    tx,
                    ProtoMessage::BluetoothDeviceConnectionResponse(
                        BluetoothDeviceConnectionResponse {
                            address: addr,
                            connected: false,
                            mtu: 0,
                            error: ERR_FAILED,
                        },
                    ),
                )
                .await;
            }
        };

        let Some(id) = self.device_id() else {
            return send(
                tx,
                ProtoMessage::BluetoothDeviceConnectionResponse(BluetoothDeviceConnectionResponse {
                    address: addr,
                    connected: false,
                    mtu: 0,
                    error: ERR_NOT_CONNECTED,
                }),
            )
            .await;
        };
        // 设备只认 0=public / 1=random；identity 类型(BLE 里是 2/3) 统一退化成 random。
        // `has_address_type=false` 时该字段无意义，必须按 public(0) 处理，不能直接取。
        let addr_type = if req.has_address_type {
            req.address_type.min(1)
        } else {
            0
        };
        let body = json!({
            "t": op,
            "addr": u64_to_mac(addr),
            "addr_type": addr_type,
        });
        let resp = match ble::gatt_call(&engines.config, &id, body, GATT_WAIT).await {
            Ok(_) => BluetoothDeviceConnectionResponse {
                address: addr,
                connected: op == "connect",
                // 设备不协商 MTU，给 BLE 默认值；客户端据此决定单次写入长度。
                mtu: if op == "connect" { 23 } else { 0 },
                error: 0,
            },
            Err(e) => BluetoothDeviceConnectionResponse {
                address: addr,
                connected: false,
                mtu: 0,
                error: error_code(&e),
            },
        };
        if resp.error != 0 {
            tracing::debug!("[esp_home] BLE {op} 失败 addr={addr:#x} err={}", resp.error);
        }
        send(tx, ProtoMessage::BluetoothDeviceConnectionResponse(resp)).await
    }

    async fn gatt_error(&self, tx: &Tx, address: u64, handle: u32, err: &BleError) -> bool {
        if !matches!(err, BleError::Unsupported) {
            tracing::debug!("[esp_home] GATT 操作失败 handle={handle}：{err:?}");
        }
        send(
            tx,
            ProtoMessage::BluetoothGattErrorResponse(BluetoothGattErrorResponse {
                address,
                handle,
                error: error_code(err),
            }),
        )
        .await
    }

    /// 周期任务：上推广播 + 转发 notify 事件。不订阅时直接返回，零开销。
    async fn poll(&mut self, engines: &Engines, tx: &Tx) -> bool {
        if !self.advert_on && self.notify.is_empty() {
            return true;
        }
        let Some(id) = self.device_id() else {
            return true;
        };
        let snap = match ble::read_snapshot(&engines.config, &id) {
            Ok(snap) => snap,
            Err(e) => {
                tracing::debug!("[esp_home] 读取 BLE 快照失败：{e:?}");
                return true;
            }
        };
        if self.advert_on && !self.push_adverts(&snap, tx).await {
            return false;
        }
        if !self.notify.is_empty() && !self.push_events(&snap, tx).await {
            return false;
        }
        true
    }

    /// 广播上推：按 `(addr, data)` 去重，内容没变就不重复发。
    async fn push_adverts(&mut self, snap: &BleSnapshot, tx: &Tx) -> bool {
        if self.seen.len() > SEEN_CAP {
            self.seen.clear();
        }
        for entry in &snap.adverts {
            let Some(advert) = to_advert(entry) else {
                continue;
            };
            let data = entry
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let key = (advert.rssi, data);
            match self.seen.get(&advert.address) {
                Some(prev) if *prev == key => continue,
                _ => {}
            }
            self.seen.insert(advert.address, key);
            if !send(tx, ProtoMessage::BluetoothLeAdvertisementResponse(advert)).await {
                return false;
            }
        }
        true
    }

    /// notify / disconnected 事件转发。
    ///
    /// 设备的事件窗口是固定容量的环形队列（最多 50 条），故用"上次最后一条在窗口里的
    /// 位置"来界定新事件；找不到（已被淘汰）就只处理窗口内的事件，宁可漏旧不可重复。
    async fn push_events(&mut self, snap: &BleSnapshot, tx: &Tx) -> bool {
        let start = match &self.last_event {
            Some(last) => snap
                .events
                .iter()
                .position(|e| e == last)
                .map_or(0, |i| i + 1),
            None => 0,
        };
        for event in &snap.events[start..] {
            self.last_event = Some(event.clone());
            let Some(addr) = event
                .get("addr")
                .and_then(Value::as_str)
                .and_then(mac_to_u64)
            else {
                continue;
            };
            match event.get("t").and_then(Value::as_str) {
                Some("notify") => {
                    let handle = event.get("handle").and_then(Value::as_u64).unwrap_or(0) as u32;
                    if !self.notify.contains(&(addr, handle)) {
                        continue;
                    }
                    let data = event
                        .get("value")
                        .and_then(Value::as_str)
                        .and_then(base64_decode)
                        .unwrap_or_default();
                    if !send(
                        tx,
                        ProtoMessage::BluetoothGattNotifyDataResponse(
                            BluetoothGattNotifyDataResponse {
                                address: addr,
                                handle,
                                data,
                            },
                        ),
                    )
                    .await
                    {
                        return false;
                    }
                }
                Some("disconnected") => {
                    // 设备主动断开：必须告诉客户端，否则它会一直以为连着。
                    self.notify.retain(|(a, _)| *a != addr);
                    if !send(
                        tx,
                        ProtoMessage::BluetoothDeviceConnectionResponse(
                            BluetoothDeviceConnectionResponse {
                                address: addr,
                                connected: false,
                                mtu: 0,
                                error: 0,
                            },
                        ),
                    )
                    .await
                    {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }
}

type Tx = mpsc::Sender<ProtoMessage>;

async fn send(tx: &Tx, msg: ProtoMessage) -> bool {
    tx.send(msg).await.is_ok()
}

/// 判断一条消息是否属于 BLE 代理（由 [`spawn`] 的任务接手）。
///
/// 主循环只做这一个判断就转发，避免慢的 GATT 操作阻塞状态回推。
pub fn is_bluetooth(msg: &ProtoMessage) -> bool {
    matches!(
        msg,
        ProtoMessage::SubscribeBluetoothLeAdvertisementsRequest(_)
            | ProtoMessage::UnsubscribeBluetoothLeAdvertisementsRequest(_)
            | ProtoMessage::BluetoothDeviceRequest(_)
            | ProtoMessage::BluetoothGattGetServicesRequest(_)
            | ProtoMessage::BluetoothGattReadRequest(_)
            | ProtoMessage::BluetoothGattWriteRequest(_)
            | ProtoMessage::BluetoothGattNotifyRequest(_)
            | ProtoMessage::BluetoothGattReadDescriptorRequest(_)
            | ProtoMessage::BluetoothGattWriteDescriptorRequest(_)
            | ProtoMessage::SubscribeBluetoothConnectionsFreeRequest(_)
    )
}

/// 为一条连接启动 BLE 代理任务。
pub fn spawn(
    engines: Arc<Engines>,
    rx: mpsc::UnboundedReceiver<ProtoMessage>,
    tx: Tx,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut proxy = Proxy::new();
        let mut rx = rx;
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // 首个 tick 会立即返回一次，白白拉一次快照；先消费掉。
        tick.tick().await;
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(msg) => {
                        if !proxy.handle(&engines, msg, &tx).await {
                            break;
                        }
                    }
                    None => break,
                },
                _ = tick.tick() => {
                    if !proxy.poll(&engines, &tx).await {
                        break;
                    }
                }
            }
        }
        tracing::debug!("[esp_home] BLE 代理会话结束");
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_roundtrip() {
        let addr = mac_to_u64("aa:bb:cc:dd:ee:ff").unwrap();
        assert_eq!(addr, 0x0000_aabb_ccdd_eeff);
        assert_eq!(u64_to_mac(addr), "aa:bb:cc:dd:ee:ff");
        // 大写、短段、缺段一律拒绝——不能拿 0 地址去连设备。
        assert_eq!(mac_to_u64("AA:BB:CC:DD:EE:FF").map(u64_to_mac), Some("aa:bb:cc:dd:ee:ff".into()));
        assert!(mac_to_u64("aa:bb:cc:dd:ee").is_none());
        assert!(mac_to_u64("aa:bb:cc:dd:ee:ff:00").is_none());
        assert!(mac_to_u64("zz:bb:cc:dd:ee:ff").is_none());
    }

    #[test]
    fn uuid_words_is_big_endian_split() {
        let words = uuid_words("0000180f-0000-1000-8000-00805f9b34fb");
        assert_eq!(words.len(), 2);
        // 前 8 字节按大端：00 00 18 0f 00 00 10 00
        assert_eq!(words[0], 0x0000_180f_0000_1000);
        assert_eq!(words[1], 0x8000_0080_5f9b_34fb);
        // 非法输入不能 panic，也不能返回半截。
        assert!(uuid_words("nope").is_empty());
    }

    #[test]
    fn short_uuid_only_for_base_uuids() {
        assert_eq!(short_uuid("0000180f-0000-1000-8000-00805f9b34fb"), 0x180f);
        assert_eq!(short_uuid("0000fe95-0000-1000-8000-00805f9b34fb"), 0xfe95);
        // 非基数 UUID：必须返回 0（客户端改用完整 uuid 字段）。
        assert_eq!(short_uuid("12345678-1234-5678-9abc-def012345678"), 0);
    }

    #[test]
    fn parse_ad_extracts_name_uuids_and_manufacturer_data() {
        // [len][type][value]：name="Dot"、16-bit uuid 0xFE95、厂商数据 company=0xFE95 + 2 字节
        // 注意 len 含 type 字节：name 段 1+3=4、uuid 段 1+2=3、厂商段 1+4=5。
        let raw = [
            0x04, 0x09, b'D', b'o', b't', // 完整本地名
            0x03, 0x03, 0x95, 0xfe, // 16-bit 服务 UUID（小端）
            0x05, 0xFF, 0x95, 0xFE, 0x01, 0x02, // 厂商数据
        ];
        let ad = parse_ad(&raw);
        assert_eq!(ad.local_name.as_deref(), Some("Dot"));
        assert_eq!(ad.service_uuids, vec![uuid16(0xfe95)]);
        assert_eq!(ad.manufacturer_data, vec![(0xfe95, vec![0x01, 0x02])]);
    }

    #[test]
    fn parse_ad_tolerates_truncated_and_garbage() {
        // 长度越过剩余缓冲：停在原地，不越界 panic。
        assert_eq!(parse_ad(&[0x09, 0x09, b'X']).local_name, None);
        // 长度为 0 表示结束，后面的字节不再解析。
        assert!(parse_ad(&[0x00, 0x03, 0x03, 0x95, 0xfe])
            .service_uuids
            .is_empty());
        // 空负载（0x16 但只有一个字节）不产出半条服务数据。
        assert!(parse_ad(&[0x02, 0x16, 0x95]).service_data.is_empty());
        assert_eq!(parse_ad(&[]), AdData::default());
    }

    #[test]
    fn parse_ad_service_data_16bit() {
        let raw = [0x05, 0x16, 0x95, 0xfe, 0xaa, 0xbb];
        let ad = parse_ad(&raw);
        assert_eq!(ad.service_data, vec![(uuid16(0xfe95), vec![0xaa, 0xbb])]);
    }

    #[test]
    fn to_advert_maps_device_json() {
        // 设备侧真实形态：addr + addrType + rssi + base64 的 data
        let entry = json!({
            "addr": "aa:bb:cc:dd:ee:ff",
            "addrType": 1,
            "rssi": -63,
            "data": base64_encode(&[0x04, 0x09, b'D', b'o', b't']),
        });
        let adv = to_advert(&entry).expect("合法 advert");
        assert_eq!(adv.address, 0x0000_aabb_ccdd_eeff);
        assert_eq!(adv.name, b"Dot".to_vec());
        assert_eq!(adv.rssi, -63);
        assert_eq!(adv.address_type, 1);
        // 缺 addr 的条目必须被丢弃，不能伪造 0 地址。
        assert!(to_advert(&json!({"rssi": -1})).is_none());
    }

    #[test]
    fn to_services_uses_value_handle_and_flags() {
        let result = json!({
            "services": [{
                "uuid": "0000180f-0000-1000-8000-00805f9b34fb",
                "start": 1,
                "end": 9,
                "chars": [{
                    "uuid": "00002a19-0000-1000-8000-00805f9b34fb",
                    "handle": 2,
                    "value_handle": 3,
                    "props": 18,
                    "descs": [{"uuid": "00002902-0000-1000-8000-00805f9b34fb", "handle": 4}],
                }],
            }],
        });
        let services = to_services(&result);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].handle, 1);
        assert_eq!(services[0].short_uuid, 0x180f);
        let ch = &services[0].characteristics[0];
        // 关键：读写要用值句柄 3，不是声明句柄 2。
        assert_eq!(ch.handle, 3);
        assert_eq!(ch.properties, 18);
        assert_eq!(ch.descriptors[0].handle, 4);
        // 服务数据缺失时返回空表而不是 panic。
        assert!(to_services(&json!({})).is_empty());
    }

    #[test]
    fn error_codes_cover_capability_and_link_states() {
        assert_eq!(error_code(&BleError::Timeout), -2);
        assert_eq!(error_code(&BleError::Busy), -4);
        assert_eq!(error_code(&BleError::NotFound), -5);
        assert_eq!(error_code(&BleError::Offline), -6);
        assert_eq!(error_code(&BleError::Unsupported), -1);
        assert_eq!(error_code(&BleError::Protocol), -1);
    }

    #[test]
    fn feature_flags_stay_honest() {
        // 只报被动扫描 + 主动连接；配对/缓存清除/原始广播一律不报。
        assert_eq!(FEATURE_FLAGS & 0b1, 0b1, "PASSIVE_SCAN");
        assert_eq!(FEATURE_FLAGS & 0b10, 0b10, "ACTIVE_CONNECTIONS");
        assert_eq!(FEATURE_FLAGS & 0b1000, 0, "不得谎报 PAIRING");
        assert_eq!(FEATURE_FLAGS & 0b1_0000, 0, "不得谎报 CACHE_CLEARING");
        assert_eq!(FEATURE_FLAGS & 0b100_0000, 0, "不得谎报 STATE_AND_MODE");
    }
}
