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
//! 服务端唯一的承载抽象是 [`crate::app::transport::Transport`]。本模块提供一个
//! [`EchomuseTransport`] 实现，把 EchoMuse 协议**翻译**成 xiaozhi 会话语义：
//!
//! | EchoMuse 事件 | 会话语义 |
//! |---|---|
//! | `oww_wake`（设备本地唤醒词命中） | `listen state=start` + 下发 `mic_start{lock_mic:true}` |
//! | `button`（动作键单击） | 同上（无唤醒词模型的兜底入口） |
//! | `0x01`/`0x07` mic PCM | 上行 Opus 二进制帧（16kHz 编码，见下） |
//! | `0x04` / `0x05` | `listen state=stop` |
//! | 下行 Opus 帧（`send_binary`） | `0x02` speaker PCM（16kHz 解码） |
//! | `tts stop` 文本 | `0x03` speaker EOS |
//!
//! 翻译在**两条 inbound 任务**里完成（control 任务 / data 任务），它们把翻译结果推入
//! 上行 channel；[`EchomuseTransport::recv`] 只是取 channel。PCM↔Opus 转换用与流水线
//! 相同的 [`crate::app::audio`] 编解码器，会话侧拿到的是标准 Opus 帧，零改动。
//!
//! ## 刻意不做的范围（靠能力协商静默关闭）
//!
//! `/shell` 面、音乐面（`audio_mix`）、BLE 代理（`ble_connect`）、Sendspin、WiFi 配置
//! 推送、TLS/wss 一概不实现：`ack.features` 只声明我们支持的能力子集，设备按协议
//! "未声明即降级"处理；未知帧型/未知 JSON 消息双向忽略，不产生错误。
//!
//! [`crate::app::audio`]: crate::app::audio

pub mod frames;
pub mod transport;

pub(crate) mod admin;
pub(crate) mod registry;
pub(crate) mod routes;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// `[echomuse]` 配置段：EchoMuse 设备入口开关与凭据。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EchomuseConfig {
    /// 总开关。默认 **false**：不启用时不挂路由、不广播 mDNS，行为与今天完全一致。
    #[serde(default)]
    pub enabled: bool,
    /// 期望的设备 token（`X-Em-Token` 头）；为空字符串表示不校验。
    /// 对应 EchoMuse 设备凭据文件里的 token（配对时由 controller 下发）。
    #[serde(default)]
    pub token: String,
}

impl Default for EchomuseConfig {
    fn default() -> Self {
        EchomuseConfig {
            enabled: false,
            token: String::new(),
        }
    }
}

impl EchomuseConfig {
    /// 规范化（幂等；与各能力配置同口径：trim 一次）。
    pub fn normalize(&mut self) {
        self.token = self.token.trim().to_string();
    }
}

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

/// 一台已通过 register 的设备的交接件，data 路由按它接管 `/em/data` 连接。
#[derive(Debug)]
pub struct PendingDevice {
    /// 上行 channel 发送端：data 任务把翻译后的会话帧推给它（会话侧消费）。
    pub up_tx: UnboundedSender<crate::app::transport::IncomingFrame>,
    /// data 面出站接收端：data 任务取走后直发设备（`0x02`/`0x03`）。
    pub data_rx: UnboundedReceiver<DataOut>,
    /// 登记时刻（`/em/status` 用它算 `connected_secs`）。
    pub since: Instant,
    /// control 出站发送端的克隆：管理面下发 `reboot`/`wifi_change` 走它
    /// （`ControlOut::Json` → control_loop → 设备）。control 会话自己持原件。
    pub ctl_tx: Option<UnboundedSender<ControlOut>>,
}

/// 进程级单例：设备 → 交接件 的登记表。
///
/// control 路由在 register 通过后登记，data 路由凭 `?device_id=`（显式）或"唯一
/// 待认领"弹出。协议要求 `/data` 与 `/control` 来自同一地址；多台设备在同一出口
/// IP（NAT）后时必须靠 device_id 消歧，真机联调再按地址收紧。
#[derive(Default)]
pub struct EchomuseHub {
    pending: Mutex<HashMap<String, PendingDevice>>,
}

static HUB: OnceLock<EchomuseHub> = OnceLock::new();

fn hub() -> &'static EchomuseHub {
    HUB.get_or_init(EchomuseHub::default)
}

/// 登记（control 侧）；同名旧记录被替换（设备重拨是常态，旧通道随任务退出关闭）。
fn hub_insert(device_id: &str, entry: PendingDevice) {
    hub()
        .pending
        .lock()
        .expect("echomuse hub 锁中毒")
        .insert(device_id.to_string(), entry);
}

/// 弹出（data 侧）。
fn hub_take(device_id: &str) -> Option<PendingDevice> {
    hub()
        .pending
        .lock()
        .expect("echomuse hub 锁中毒")
        .remove(device_id)
}

/// 当前登记快照（只读；`hub_take_only` 借它判断"唯一待认领"）。
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

/// 会话结束后的清理（control 任务退出时调用；不存在则忽略）。
fn hub_remove(device_id: &str) {
    hub()
        .pending
        .lock()
        .expect("echomuse hub 锁中毒")
        .remove(device_id);
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
}

/// 服务器 → 设备 的 `ack`（能力协商：`features` 是**服务器**的能力表，
/// 未列出的能力设备按协议降级——`listen_session` 不列出即不会走 `0x07` 私听会话，
/// 统一走 `mic_start{lock_mic:true}` bounded turn，服务端只处理 `0x01`/`0x04`/`0x05`）。
pub fn ack_message(device_id: &str) -> String {
    serde_json::json!({
        "type": "ack",
        "device_id": device_id,
        "features": ["mic", "speaker", "leds", "buttons", "oww_trigger", "oww_local_only"],
    })
    .to_string()
}

/// 服务器 → 设备 的极简 `config` 推送：只写行为开关，不碰设备音频链路参数
/// （EQ/AEC/波束等保持设备存储值）。`owwOnDevice=true` 让唤醒词在设备本地识别，
/// 服务端零持续音频负载。
///
/// `settings` 是管理面保存的每设备部分更新（协议 camelCase 字段，已在 admin 层
/// 白名单过滤）：非 `null` 键覆盖基线，协议本身是"零/缺省字段忽略"的部分更新。
/// `wakeWordEnabled` 按协议**单独一条** config 发送（HA 的 per-Echo 拾取态，不与
/// 存储配置混发）。返回按序发送的 JSON 文本列表（1~2 条）。
pub fn config_push_with(settings: &serde_json::Value) -> Vec<String> {
    let mut base = serde_json::json!({
        "type": "config",
        "owwOnDevice": true,
        "streamReply": true,
    });
    if let (Some(obj), Some(base_obj)) = (settings.as_object(), base.as_object_mut()) {
        for (k, v) in obj {
            if k == "wakeWordEnabled" {
                continue;
            }
            base_obj.insert(k.clone(), v.clone());
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
        // 广播地址：取默认路由出口的局域网 IPv4；失败退回 loopback（容器内常见，仅日志可见）。
        let ip = local_ipv4().unwrap_or(Ipv4Addr::LOCALHOST);
        let props = [("plain_port".to_string(), port.to_string())]
            .into_iter()
            .collect();
        let info = match ServiceInfo::new(
            "_emcontroller._tcp.local.",
            "xiaozhi-server",
            "xiaozhi-server.local.",
            format!("{ip}:{port}"),
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

/// 探测本机默认出口的局域网 IPv4（UDP connect 不发包，只查路由表）。
fn local_ipv4() -> Option<Ipv4Addr> {
    let sock = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    sock.connect(("8.8.8.8", 80)).ok()?;
    match sock.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) => Some(v4),
        std::net::IpAddr::V6(_) => None,
    }
}

/// 会话参数：EchoMuse 侧固定 16k/60ms/v1——mic 是 16k PCM（协议常量），TTS 解码后
/// 直接按 16k 出 PCM 给设备，无需重采样；二进制协议 v1（裸 Opus，无头）。
pub(crate) fn session_params(cfg: &crate::config::Config) -> crate::app::session::SessionParams {
    crate::app::session::SessionParams {
        uplink_bin_ver: crate::app::protocol::BinVersion::V1,
        downlink_bin_ver: crate::app::protocol::BinVersion::V1,
        uplink_sr: frames::MIC_SAMPLE_RATE,
        downlink_sr: frames::MIC_SAMPLE_RATE,
        downlink_frame_ms: 60,
        downlink_lead_ms: cfg.audio.downlink_lead_ms,
        downlink_buffer_ms: cfg.audio.downlink_buffer_ms,
        download_buffer_ms: cfg.audio.download_budget_ms(),
        downlink_buffer_start_ms: cfg.audio.downlink_buffer_start_ms,
        downlink_buffer_start_max_ms: cfg.audio.downlink_buffer_start_max_ms,
    }
}
