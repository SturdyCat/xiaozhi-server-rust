//! EchoMuse 设备入口：Echo Dot 2 刷 emOS 后经 EchoMuse 设备协议直接接入本服务端。
//!
//! ## 协议来源与边界
//!
//! 设备侧协议权威来源是 EchoMuse 仓库 `docs/device-controller-interface.md`：
//! 设备经 mDNS `_emcontroller._tcp.local` 发现 controller，开两条本项目用到的
//! WebSocket——`/em/control`（JSON：register/ack 能力协商、唤醒/按钮事件、配置推送）
//! 与 `/em/data`（二进制：mic PCM 上行、speaker PCM 下行，帧布局见 [`frames`]）。
//! 鉴权是 `X-Em-Token` 头；无 CA 的设备拨 `ws://` 明文（首次配对场景），故首版不做 TLS。
//!
//! ## 与 xiaozhi 会话流水线的桥接
//!
//! 服务端唯一的承载抽象是 [`crate::workflow::protocol::Transport`]。本模块提供一个
//! [`EchomuseTransport`] 实现，把 EchoMuse 协议**翻译**成 xiaozhi 会话语义：
//!
//! | EchoMuse 事件 | 会话语义 |
//! |---|---|
//! | `oww_wake`（设备本地唤醒词命中） | `listen detect` → `mic_start{lock_mic:true}` → `listen start` |
//! | `button`（动作键单击） | 同上（无唤醒词模型的兜底入口） |
//! | `0x01`/`0x07` mic PCM | 上行 Opus 二进制帧（16kHz 编码，见下） |
//! | `0x04` / `0x05` | typed `EndOfUtterance(Speech / NoSpeech)`，立即允许下一轮 |
//! | 下行 Opus 帧（`send_binary`） | `0x02` speaker PCM（16kHz 解码） |
//! | `tts stop` 文本 | `0x03` speaker EOS |
//!
//! 翻译在**两条 inbound 任务**里完成（control 任务 / data 任务），它们把翻译结果推入
//! 上行 channel；[`EchomuseTransport::recv`] 只是取 channel。PCM↔Opus 转换用与流水线
//! 相同的 [`crate::media`] 编解码器，会话侧拿到的是标准 Opus 帧，零改动。
//!
//! ## 蓝牙代理（`ble_connect` + `ble_adverts_data`，`[echomuse].ble_proxy` 开启时）
//!
//! 设备侧 BLE 扫描与 GATT 连接桥的服务端承载：广播批次经 `0x06` 帧上报进
//! [`ble::BleLink`] 合并表，GATT 请求/结果经 `0x08` 帧与设备互通（`req` 关联），
//! 管理面 `/api/echomuse/devices/{id}/ble/*` 对外。仅当 ack 声明对应能力且设备
//! register 也声明 `ble_connect` 时启用；关闭时零行为变化。协议契约见根文档与
//! EchoMuse `device-controller-interface.md`。
//!
//! ## 刻意不做的范围（靠能力协商静默关闭）
//!
//! `/shell/{device_id}` 仅承载已批准 emOS 的固定系统诊断/确认重启，不提供交互
//! shell、任意命令或 OTA。音乐面（`audio_mix`）、Sendspin、TLS/wss 不实现；
//! WiFi 保留既有 control 命令下发（排队不等于设备已应用）。
//! `ack.features` 只声明我们支持的能力子集，设备按协议"未声明即降级"处理；
//! 未知帧型/未知 JSON 消息双向忽略，不产生错误。蓝牙代理只做**直驱 GATT + 广播
//! 采集**，不做 HA/ESPHome 转发面，也不做配对/加密（协议规定 Security Request 答
//! Pairing Not Supported，加密属性以 `att:5` 失败）。
//!
//! [`crate::media`]: crate::media

pub mod ble;
pub mod frames;
pub mod transport;

pub(crate) mod admin;
#[cfg(test)]
mod admin_tests;
#[cfg(test)]
mod ble_tests;
mod config;
#[cfg(test)]
mod hub_tests;
pub(crate) mod listen;
#[cfg(test)]
mod listen_tests;
mod onnx_typed;
pub(crate) mod oww;
pub(crate) mod registry;
pub(crate) mod routes;
pub(crate) mod runtime;
mod settings;
pub(crate) mod shell;
pub(crate) mod system;
#[cfg(test)]
mod tests;
mod turn;
mod uplink;

pub use config::{connect_welcome, EchomuseConfig};

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use serde::Deserialize;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;

/// control 面出站消息：要么是 JSON 文本，要么是"断开"指令。
// `Json` 的生产路径：会话层 mic_start（control 任务内直发）与管理面 command 下发
// （admin.rs 经 hub 登记的 ctl_tx 克隆）。
#[derive(Debug)]
pub enum ControlOut {
    /// 一条 JSON 文本（mic_start / listen_ack / led_anim / config 等）。
    Json(String),
    /// 结束 control 任务并关闭连接（会话主动 close 时）。
    Shutdown,
}

/// data 面出站消息：已完整组帧的字节（`0x02`/`0x03`）。
pub type DataOut = Vec<u8>;
mod data_writer;
pub(crate) mod media;

/// 一台已通过 register 的设备的交接件，登记在 hub 里存活到会话结束；data 路由
/// 只认领其中的 [`PendingDevice::data_rx`]（认领后 data 任务退出时经
/// [`hub_restore_data`] 归还，设备重拨 `/em/data` 不必等下次 register），其余字段
/// （control/data 出站口、蓝牙代理链路）供管理面在设备完全在线期间随时使用。
#[derive(Debug)]
pub struct PendingDevice {
    /// 上行 channel 发送端：data 任务把翻译后的会话帧推给它（会话侧消费）。
    pub up_tx: UnboundedSender<crate::workflow::protocol::IncomingFrame>,
    /// data 面出站接收端：data 路由认领时取走（`hub_claim_data`），认领后置 None。
    pub data_rx: Option<UnboundedReceiver<DataOut>>,
    /// 登记时刻（`/em/status` 用它算 `connected_secs`）。
    pub since: Instant,
    /// control 出站发送端的克隆：管理面下发 `volume_set`/`shell_open`/`wifi_change` 走它
    /// （`ControlOut::Json` → control_loop → 设备）。control 会话自己持原件。
    pub ctl_tx: Option<UnboundedSender<ControlOut>>,
    /// data 面出站发送端的克隆：蓝牙代理 GATT 请求（`0x08`）经它下发
    /// （管理面 → hub → data 任务 → 设备）。
    pub data_tx: Option<UnboundedSender<DataOut>>,
    /// 蓝牙代理链路状态（请求关联 + 广播/事件采集）；会话结束随条目消失。
    pub ble: Arc<Mutex<ble::BleLink>>,
    /// Current control TCP identity and bounded telemetry (not persisted).
    pub(crate) management: Arc<runtime::LiveControl>,
}

/// data 路由认领 `data_rx` 后 data 任务需要的全部交接件；任务退出时整包经
/// [`hub_restore_data`] 归还（设备重拨 `/em/data` 立即可再认领，不必等下次 register）。
pub(crate) struct DataClaim {
    /// 所属设备（归还时定位 hub 条目）。
    pub device_id: String,
    /// 上行发送端（data 任务把翻译后的会话帧推入）。
    pub up_tx: UnboundedSender<crate::workflow::protocol::IncomingFrame>,
    /// data 面出站接收端（已从条目取走）。
    pub data_rx: UnboundedReceiver<DataOut>,
    /// data 面出站发送端探针：归还时校验 hub 条目仍是本会话（未被新 register 替换）。
    pub data_tx: UnboundedSender<DataOut>,
    /// 蓝牙代理链路（data 任务收发 `0x06`/`0x08` 用；与 hub 条目同享）。
    pub ble: Arc<Mutex<ble::BleLink>>,
    /// Captured control identity; never reacquire the listening gate by device id.
    pub management: Arc<runtime::LiveControl>,
}

/// 进程级单例：设备 → 交接件 的登记表。
///
/// control 路由在 register 通过后登记，条目**存活到会话结束**（`hub_remove` 清理）；
/// data 路由凭 `?device_id=`（显式）或"唯一未认领"取出 [`DataClaim`]。协议要求
/// `/data` 与 `/control` 来自同一地址；多台设备在同一出口 IP（NAT）后时必须靠
/// device_id 消歧，真机联调再按地址收紧。
#[derive(Default)]
pub struct EchomuseHub {
    pending: Mutex<HashMap<String, PendingDevice>>,
}

static HUB: OnceLock<EchomuseHub> = OnceLock::new();

fn hub() -> &'static EchomuseHub {
    HUB.get_or_init(EchomuseHub::default)
}

/// 设备面变化版本号（进程级 watch 通道）：hub 登记/移除与管理面登记变更时自增。
/// 管理台经 WebSocket `GET /api/admin/ws` 实时订阅它（`gateway::ws::admin_ws`：hello
/// 带当前版本、变化推 `echomuse_devices_changed`），`/devices/wait` 长挂轮询已删除。
static DEVICES_VERSION: OnceLock<watch::Sender<u64>> = OnceLock::new();

fn devices_version_tx() -> &'static watch::Sender<u64> {
    DEVICES_VERSION.get_or_init(|| watch::channel(0).0)
}

/// 当前设备面版本号（`devices` 列表回包携带，与管理台 WS hello/事件帧同源）。
pub(crate) fn devices_version() -> u64 {
    *devices_version_tx().borrow()
}

/// 通知"设备面已变化"（hub 登记/移除、批准/设置/删除登记后调用）。
/// `send_modify` 在写锁内递增，无订阅者也更新；不能拆成 borrow +
/// send_replace，否则并发事件可能丢增量，旧快照会拿到已消费的新游标。
pub(crate) fn bump_devices_version() {
    devices_version_tx().send_modify(|version| *version = version.wrapping_add(1));
}

/// 订阅变化通知（管理台 WS `gateway::ws::admin_ws` 推送用；watch 惰性初始化，订阅前
/// 已发生的变化用版本号数值对齐，不依赖通道回放）。
pub(crate) fn subscribe_devices_version() -> watch::Receiver<u64> {
    devices_version_tx().subscribe()
}

/// 登记（control 侧）；同名旧记录被替换（设备重拨是常态，旧通道随任务退出关闭）。
fn hub_insert(device_id: &str, entry: PendingDevice) {
    if let Some(old) = hub()
        .pending
        .lock()
        .expect("echomuse hub 锁中毒")
        .insert(device_id.to_string(), entry)
    {
        old.management.media.close();
        let _ = old.up_tx.send(crate::workflow::protocol::IncomingFrame::Closed);
    }
    // 上线/下线对管理台列表可见（online / connected_secs）。
    bump_devices_version();
}

/// 认领 data 面出站接收端（已认领/不存在返回 None），条目保留。
pub(crate) fn hub_claim_data(device_id: &str) -> Option<DataClaim> {
    let mut guard = hub_iter();
    let entry = guard.get_mut(device_id)?;
    let data_rx = entry.data_rx.take()?;
    Some(build_claim(device_id.to_string(), entry, data_rx))
}

/// 恰好一台 data 未认领时取它；零台或多台都返回 None（多台必须显式 device_id）。
pub(crate) fn hub_claim_only() -> Option<DataClaim> {
    let mut guard = hub_iter();
    let mut found: Option<String> = None;
    for (id, entry) in guard.iter() {
        if entry.data_rx.is_some() {
            if found.is_some() {
                return None;
            }
            found = Some(id.clone());
        }
    }
    let id = found?;
    let entry = guard.get_mut(&id)?;
    let data_rx = entry.data_rx.take()?;
    Some(build_claim(id, entry, data_rx))
}

/// 取走 data 接收端后为指定设备构造 [`DataClaim`]（两个认领入口共用）：
/// 认领媒体收发、克隆 GATT 下发口，其余出站口/蓝牙链路与管理面引用一并转出。
fn build_claim(
    device_id: String,
    entry: &mut PendingDevice,
    data_rx: UnboundedReceiver<DataOut>,
) -> DataClaim {
    entry.management.media.claim();
    let data_tx = entry
        .data_tx
        .clone()
        .expect("register 构造的条目 data_tx 必在");
    DataClaim {
        device_id,
        up_tx: entry.up_tx.clone(),
        data_rx,
        data_tx,
        ble: entry.ble.clone(),
        management: entry.management.clone(),
    }
}

/// Return only the owning data claim. Reset BLE and discard frames queued before
/// disconnect under the same hub lock used by GATT dispatch, before making the
/// receiver claimable again: old writes must never replay on a replacement WS.
/// Lock order is hub -> BLE, never BLE -> hub; there is no await in this section.
pub(crate) fn hub_restore_data(
    device_id: &str,
    mut data_rx: UnboundedReceiver<DataOut>,
    probe: &UnboundedSender<DataOut>,
) {
    let mut guard = hub_iter();
    if let Some(entry) = guard.get_mut(device_id) {
        let same_session = entry
            .data_tx
            .as_ref()
            .is_some_and(|tx| tx.same_channel(probe));
        if same_session && entry.data_rx.is_none() {
            entry.management.media.disconnect();
            // Data loss cancels only this control's capture before a re-claim.
            entry.management.turn.finish(
                crate::workflow::protocol::UtteranceEnd::NoSpeech,
                &entry.up_tx,
                |_| {},
            );
            entry.ble.lock().expect("echomuse ble link 锁中毒").reset();
            // Snapshot the length so concurrent audio producers cannot keep this
            // bounded cleanup running forever. GATT producers require hub lock.
            let queued = data_rx.len();
            for _ in 0..queued {
                let _ = data_rx.try_recv();
            }
            entry.data_rx = Some(data_rx);
        }
    }
}

/// 当前登记快照（只读）。
fn hub_iter() -> std::sync::MutexGuard<'static, HashMap<String, PendingDevice>> {
    hub().pending.lock().expect("echomuse hub 锁中毒")
}

/// 当前登记设备的 `(device_id, connected_secs)` 快照（`/em/status` 与管理面
/// devices 列表合并 online 用；hub 的 key = 处于会话中的在线设备，会话结束
/// `hub_remove` 后自然消失）。
pub(crate) fn hub_status() -> Vec<(String, u64)> {
    hub_iter()
        .iter()
        .map(|(id, pending)| (id.clone(), pending.since.elapsed().as_secs()))
        .collect()
}

/// 某设备 control 出站口的克隆（管理面下发 command 用）；不在线（不在 hub）返回 None。
pub(crate) fn hub_ctl(device_id: &str) -> Option<UnboundedSender<ControlOut>> {
    hub_iter().get(device_id).and_then(|p| p.ctl_tx.clone())
}

/// Atomically capture control channel identity and its telemetry/peer metadata.
pub(crate) fn hub_live(
    device_id: &str,
) -> Option<(UnboundedSender<ControlOut>, Arc<runtime::LiveControl>)> {
    let guard = hub_iter();
    let entry = guard.get(device_id)?;
    Some((entry.ctl_tx.clone()?, entry.management.clone()))
}

/// Only the owning control task may remove a hub entry (reconnect-safe).
pub(crate) fn hub_remove_session(device_id: &str, ctl: &UnboundedSender<ControlOut>) {
    let removed = {
        let mut guard = hub_iter();
        if guard
            .get(device_id)
            .and_then(|p| p.ctl_tx.as_ref())
            .is_some_and(|c| c.same_channel(ctl))
        {
            guard.remove(device_id);
            true
        } else {
            false
        }
    };
    if removed {
        bump_devices_version();
    }
}

/// Fixture-only observation of the data sender. Production BLE dispatch captures
/// control/session/data/link together; a separate lookup by id is reconnect-unsafe.
#[cfg(test)]
pub(crate) fn hub_data(device_id: &str) -> Option<UnboundedSender<DataOut>> {
    hub_iter().get(device_id).and_then(|p| p.data_tx.clone())
}

/// Fixture-only BLE observation. Production dispatch uses its atomic capture;
/// lifecycle sections acquire hub -> BLE, never the reverse order.
#[cfg(test)]
pub(crate) fn hub_ble<T>(device_id: &str, f: impl FnOnce(&mut ble::BleLink) -> T) -> Option<T> {
    let link = {
        let guard = hub_iter();
        guard.get(device_id)?.ble.clone()
    };
    let mut link = link.lock().expect("echomuse ble link 锁中毒");
    Some(f(&mut link))
}

/// `GET /em/status` 的响应体（与 HTTP 层解耦，便于单测直接断言形状）。
pub(crate) fn status_json(enabled: bool) -> serde_json::Value {
    let devices: Vec<serde_json::Value> = hub_status()
        .into_iter()
        .map(|(device_id, connected_secs)| {
            serde_json::json!({ "device_id": device_id, "connected_secs": connected_secs })
        })
        .collect();
    serde_json::json!({ "enabled": enabled, "devices": devices })
}

/// 测试 fixture 清理；生产路径必须使用带会话探针的 hub_remove_session。
#[cfg(test)]
fn hub_remove(device_id: &str) {
    let removed = hub()
        .pending
        .lock()
        .expect("echomuse hub 锁中毒")
        .remove(device_id)
        .is_some();
    if removed {
        // 下线对管理台列表可见（online 消失）。
        bump_devices_version();
    }
}

/// 设备 → 服务器 的 register 负载（只取本项目用到的字段，其余忽略——协议前向兼容）。
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceRegister {
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub board: Option<String>,
    #[serde(default)]
    pub base_os: Option<String>,
    /// 配对请求（上游固件：装过 CA 的设备在无 tls_port 时按住 action 键降级
    /// plain 拨号并置位；该拨号**不带 token**，见 routes.rs 的鉴权豁免）。
    #[serde(default)]
    pub pairing: bool,
}

/// 服务器 → 设备 的 `ack`（能力协商：`features` 是**服务器**的能力表，
/// 未列出的能力设备按协议降级——`listen_session` 不列出即不会走 `0x07` 私听会话，
/// 统一走 `mic_start{lock_mic:true}` bounded turn，服务端只处理 `0x01`/`0x04`/`0x05`）。
///
/// `ble=true` 时追加 `ble_connect` + `ble_adverts_data`：调用方（control 接入）仅在
/// `[echomuse].ble_proxy` 开启**且**设备 register 声明 `ble_connect` 时置位（协议
/// 要求双向声明才启用，设备按"未声明即降级"静默关闭）。
pub fn ack_message(device_id: &str, ble: bool) -> String {
    let mut features = vec![
        "mic",
        "speaker",
        "leds",
        "buttons",
        "oww_trigger",
        "oww_local_only",
    ];
    if ble {
        features.push("ble_connect");
        features.push("ble_adverts_data");
    }
    serde_json::json!({
        "type": "ack",
        "device_id": device_id,
        "features": features,
    })
    .to_string()
}

/// control 接入时的 config 基线：`ble_proxy` 开启时默认打开设备侧扫描与连接
/// （协议键 `bleProxyEnabled`/`bleProxyConnections`，两键都在设备才会行动），
/// 每设备保存的 settings 覆盖基线（协议部分更新语义，零/缺省字段被忽略）。
pub(crate) fn config_baseline(ble_proxy: bool, settings: &serde_json::Value) -> serde_json::Value {
    let mut eff = serde_json::Map::new();
    if ble_proxy {
        eff.insert("bleProxyEnabled".into(), serde_json::Value::from(true));
        eff.insert("bleProxyConnections".into(), serde_json::Value::from(true));
    }
    if let Some(obj) = settings.as_object() {
        for (k, v) in obj {
            eff.insert(k.clone(), v.clone());
        }
    }
    serde_json::Value::Object(eff)
}

/// 服务器 → 设备 的极简 `config` 推送：只写行为开关，不碰设备音频链路参数
/// （EQ/AEC/波束等保持设备存储值）。`owwOnDevice="on"` 让唤醒词在设备本地识别，
/// 服务端零持续音频负载。
///
/// `settings` 是管理面保存的每设备部分更新（协议 camelCase 字段，已在 admin 层
/// 白名单过滤）：非 `null` 键覆盖基线，协议本身是"零/缺省字段忽略"的部分更新。
/// `wakeWordEnabled` 按协议**单独一条** config 发送（HA 的 per-Echo 拾取态，不与
/// 存储配置混发）。返回按序发送的 JSON 文本列表（1~2 条）。
pub fn config_push_with(settings: &serde_json::Value) -> Vec<String> {
    let mut base = serde_json::json!({
        "type": "config",
        "owwOnDevice": "on",
        "streamReply": true,
    });
    if let (Some(obj), Some(base_obj)) = (settings.as_object(), base.as_object_mut()) {
        for (k, v) in obj {
            if k == "wakeWordEnabled" {
                continue;
            }
            if let Some(value) = settings::wire_value(k, v) {
                base_obj.insert(k.clone(), value);
            }
        }
    }
    let mut out = vec![base.to_string()];
    if let Some(b) = settings.get("wakeWordEnabled") {
        out.push(serde_json::json!({ "type": "config", "wakeWordEnabled": b }).to_string());
    }
    out
}

/// 开机广播 mDNS `_emcontroller._tcp.local`（`[echomuse].enabled=true` 时由 main 调）。
///
/// 阻塞细节（网络注册）放在 `spawn_blocking`；失败只告警不退出——mDNS 只是发现
/// 辅助，设备凭据文件里已有 endpoint 时不需要它。
pub fn spawn_mdns(port: u16) {
    let ip = advertise_ipv4();
    tokio::task::spawn_blocking(move || {
        use mdns_sd::{ServiceDaemon, ServiceInfo};

        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    "[echomuse] mDNS daemon 启动失败（设备仍可用 endpoint 文件直连）: {e}"
                );
                return;
            }
        };
        // 广播地址：动态探测部署机默认路由出口的局域网 IPv4，
        // 失败退回 loopback（容器内常见，仅日志可见）。
        let props = [("plain_port".to_string(), port.to_string())]
            .into_iter()
            .collect();
        let info = match ServiceInfo::new(
            "_emcontroller._tcp.local.",
            "xiaozhi-server",
            "xiaozhi-server.local.",
            // mdns-sd 0.11 只接受纯 IP 字符串（端口由下一参单独给出），
            // 传 "ip:port" 会报 invalid IP address syntax
            ip.to_string(),
            port,
            Some(props),
        ) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!("[echomuse] mDNS ServiceInfo 构造失败: {e}");
                return;
            }
        };
        match daemon.register(info) {
            Ok(_rx) => {
                tracing::info!("[echomuse] mDNS 已广播 _emcontroller._tcp（{ip}:{port}）");
            }
            Err(e) => tracing::warn!("[echomuse] mDNS 注册失败: {e}"),
        }
    });
}

/// 解析 mDNS 广播地址：动态探测部署机默认出口的局域网 IPv4（部署在哪台机器、
/// 是什么 IP 就广播什么，不做任何硬编码/配置）；探测失败退回 loopback 并告警。
fn advertise_ipv4() -> Ipv4Addr {
    match local_ipv4() {
        Some(ip) => ip,
        None => {
            tracing::warn!(
                "[echomuse] 无法探测局域网 IPv4（容器部署常见），mDNS 退回 127.0.0.1；\
                 设备发现不了时请改用 host 网络部署"
            );
            Ipv4Addr::LOCALHOST
        }
    }
}

/// 探测本机默认出口的局域网 IPv4：实现移到 [`crate::device::local_ipv4`]（与
/// macApp 连接地址展示共用），此处保留薄封装以维持测试入口。
fn local_ipv4() -> Option<Ipv4Addr> {
    super::local_ipv4()
}

/// Native Echo PCM geometry; no ESP codec/wire negotiation occurs.
pub(crate) fn session_params(cfg: &crate::config::Config) -> crate::workflow::session::SessionParams {
    crate::workflow::session::SessionParams::configured_defaults(
        cfg,
        crate::media::media::AudioCodec::PcmS16Le,
    )
}
