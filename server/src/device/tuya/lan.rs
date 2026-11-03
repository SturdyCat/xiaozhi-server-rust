//! 局域网会话池（rustuya **async facade**）+ 控制失败三归类 + DP 载荷解析。
//!
//! ## rustuya 使用纪律（本 crate 的唯一使用面）
//!
//! - **只允许 async facade**（[`rustuya::Device`] / [`rustuya::DeviceBuilder`] /
//!   [`rustuya::Scanner`]）——`rustuya::sync::Device` 在 tokio runtime 内会 panic
//!   （内部 `blocking_send`），本模块绝不 import `rustuya::sync`；
//! - `Device::build()` 立即 spawn 后台连接任务（自动重连 + 心跳 + IP 重发现都在库内），
//!   最后一个 clone drop 或 `stop()` 后任务退出——会话池按 device_id 持 `Arc<Device>`，
//!   local_key 变更 / ip 变化时 `stop()` 旧会话再重建；
//! - 会话**懒式建连**：首次控制/轮询才 `DeviceBuilder::new(id, key)
//!   .address(ip).timeout(5s).build()`，服务重启后首个动作自动重连；
//! - **日志脱敏**：本模块全部日志只记命令类型与载荷长度，禁止输出 local_key 与原始报文。
//!
//! ## 失败归类（架构 §4）
//!
//! | kind | 判定 | UI 建议动作 |
//! |---|---|---|
//! | offline | 库内 online=0 / connect refused / unreachable | 检查供电与网络；可尝试重新同步 |
//! | timeout | 请求超时无响应 | 稍后重试；频发则检查 2.4G 网络 |
//! | unsupported | 协议握手失败/版本不支持/local_key 错（解密失败） | 重新同步获取 local_key；确认协议版本 |

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustuya::device::Device;
use rustuya::error::TuyaError;

use super::store::{DeviceRow, TuyaStore};

/// 单请求超时（DeviceBuilder.timeout；3–5s 区间取上限覆盖弱网握手）。
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// 包住库调用的外层超时（builder 超时之外再兜一层，防库内任务挂死拖住 API）。
const CALL_TIMEOUT: Duration = Duration::from_secs(6);

/// 控制失败三归类（含 UI 建议动作文案；`kind` ∈ offline/timeout/unsupported）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlFailure {
    pub kind: &'static str,
    pub message: String,
    pub advice: &'static str,
}

impl ControlFailure {
    fn new(kind: &'static str, message: String, advice: &'static str) -> Self {
        ControlFailure { kind, message, advice }
    }

    /// 库内预检/无 ip 的离线失败（不经过 TuyaError）。
    pub fn offline(message: String) -> Self {
        ControlFailure::new("offline", message, "检查设备供电与网络；可尝试「立即同步」更新设备目录")
    }

    /// HTTP 状态码映射：offline→409 / timeout→504 / unsupported→400。
    pub fn status_code(&self) -> u16 {
        match self.kind {
            "timeout" => 504,
            "unsupported" => 400,
            _ => 409,
        }
    }
}

/// rustuya 错误 → 三归类（纯函数，lan_tests 直驱）。
pub(crate) fn classify(err: &TuyaError) -> ControlFailure {
    let text = err.to_string();
    match err {
        // 显式超时（库内 5s 无响应）
        TuyaError::Timeout => ControlFailure::new(
            "timeout",
            "设备无响应（超时）".into(),
            "稍后重试；若频发请检查设备所在 2.4G 网络",
        ),
        TuyaError::Io { kind, .. } => {
            if *kind == std::io::ErrorKind::TimedOut {
                ControlFailure::new(
                    "timeout",
                    format!("设备无响应（{text}）"),
                    "稍后重试；若频发请检查设备所在 2.4G 网络",
                )
            } else {
                // ConnectionRefused / HostUnreachable / NetworkUnreachable / 其他 IO
                ControlFailure::offline(format!("无法连接设备（{text}）"))
            }
        }
        TuyaError::Offline | TuyaError::ConnectionFailed | TuyaError::DeviceNotFound(_) => {
            ControlFailure::offline(format!("设备离线或不可达（{text}）"))
        }
        // 协议/密钥面错误：握手失败、版本不支持、local_key 错（解密/HMAC 失败）
        TuyaError::HandshakeFailed
        | TuyaError::KeyOrVersionError
        | TuyaError::HmacMismatch
        | TuyaError::DecryptionFailed
        | TuyaError::EncryptionFailed
        | TuyaError::InvalidHeader
        | TuyaError::CrcMismatch
        | TuyaError::InvalidPayload => ControlFailure::new(
            "unsupported",
            format!("协议握手或版本不支持（{text}）"),
            "请「立即同步」获取最新 local_key；确认设备协议版本",
        ),
        // 解析/广播类：罕见，按不支持归类兜底（三类是 UI 的完整口径）
        TuyaError::Json(_) | TuyaError::DecodeError(_) | TuyaError::BroadcastLagged { .. } => {
            ControlFailure::new(
                "unsupported",
                format!("设备响应无法解析（{text}）"),
                "请「立即同步」更新设备目录后重试",
            )
        }
    }
}

/// DP 载荷 → `dp_id → value` 映射（纯函数）。
///
/// 涂鸦报文两种形状都见过：裸 `{"1":true,"2":30}` 与包裹 `{"dps":{"1":true}}`。
pub(crate) fn parse_dps(payload: &str) -> HashMap<String, serde_json::Value> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
        return HashMap::new();
    };
    let obj = match v.get("dps").filter(|d| d.is_object()) {
        Some(dps) => dps,
        None => &v,
    };
    obj.as_object()
        .map(|m| m.iter().map(|(k, val)| (k.clone(), val.clone())).collect())
        .unwrap_or_default()
}

/// 局域网事件汇：listener 消息（设备主动推送）经它回写 DP 缓存与在线状态。
#[derive(Clone)]
pub(crate) struct LanSink {
    pub store: Arc<TuyaStore>,
    pub dp_cache: Arc<Mutex<HashMap<String, HashMap<String, serde_json::Value>>>>,
}

impl LanSink {
    /// listener 消息到达：更新 DP 内存缓存 + online=1（事件修正，不必等 60s 轮询）。
    /// 日志只记 dp 数量，不记载荷内容。
    pub(crate) fn on_message(&self, device_id: &str, payload: Option<String>) {
        let Some(text) = payload else { return };
        let dps = parse_dps(&text);
        if dps.is_empty() {
            return;
        }
        let count = dps.len();
        self.dp_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(device_id.to_string(), dps);
        if let Err(e) = self.store.set_online(device_id, true, None) {
            tracing::warn!("Tuya 设备 {device_id} 在线状态写库失败: {e}");
        }
        tracing::debug!("Tuya 设备 {device_id} 推送 {count} 个 DP 状态");
    }
}

struct Session {
    device: Arc<Device>,
    local_key: String,
    ip: String,
    /// listener 消费任务是否已挂（每会话一次，防重复 spawn）。
    listening: bool,
}

/// 会话池：device_id → 懒建的 rustuya 会话（自动重连/心跳由库负责）。
pub(crate) struct LanPool {
    sessions: Mutex<HashMap<String, Session>>,
    sink: LanSink,
}

impl LanPool {
    pub(crate) fn new(sink: LanSink) -> Self {
        LanPool { sessions: Mutex::new(HashMap::new()), sink }
    }

    /// 取（或按需建）会话。local_key 变化 → 停旧建新。
    pub(crate) async fn get_or_build(&self, row: &DeviceRow) -> Result<Arc<Device>, ControlFailure> {
        if let Some(session) = self.reusable(&row.device_id, &row.local_key) {
            return Ok(session);
        }
        // 旧会话（key 变了）停掉：库内任务随最后一个 clone + stop() 退出
        self.drop_session(&row.device_id);
        if row.ip.is_empty() {
            return Err(ControlFailure::offline(
                "设备没有局域网地址（尚未被发现；请稍候或点「立即同步」）".into(),
            ));
        }
        tracing::info!("Tuya 设备 {} 建立局域网会话（ip={}，协议自动协商）", row.device_id, row.ip);
        let device = Arc::new(
            rustuya::DeviceBuilder::new(row.device_id.clone(), row.local_key.clone().into_bytes())
                .address(row.ip.clone())
                .timeout(REQUEST_TIMEOUT)
                .build(),
        );
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                row.device_id.clone(),
                Session { device: device.clone(), local_key: row.local_key.clone(), ip: row.ip.clone(), listening: false },
            );
        Ok(device)
    }

    fn reusable(&self, device_id: &str, local_key: &str) -> Option<Arc<Device>> {
        let guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get(device_id)
            .filter(|s| s.local_key == local_key)
            .map(|s| s.device.clone())
    }

    /// 停掉并移除某设备会话（local_key 变更 / 显式重建时）。
    pub(crate) fn drop_session(&self, device_id: &str) {
        let session = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(device_id);
        if let Some(session) = session {
            session.device.fire_stop();
        }
    }

    /// 发现到的 ip 与现会话不一致 → 丢弃会话（下次动作按新 ip 重建）。
    pub(crate) fn drop_if_ip_differs(&self, device_id: &str, ip: &str) {
        let differs = {
            let guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .get(device_id)
                .is_some_and(|s| !ip.is_empty() && s.ip != ip)
        };
        if differs {
            self.drop_session(device_id);
        }
    }

    /// 为尚未挂 listener 的会话补挂消费任务（事件修正在线状态与 DP 缓存）。
    pub(crate) fn ensure_listener(&self, device_id: &str) {
        let device = {
            let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            let Some(session) = guard.get_mut(device_id) else { return };
            if session.listening {
                return;
            }
            session.listening = true;
            session.device.clone()
        };
        let sink = self.sink.clone();
        let id = device_id.to_string();
        tokio::spawn(async move {
            let mut stream = device.listener();
            // listener 流持有强引用：任务活着 = 会话任务活着；池 drop 后由 stop() 收尾
            while let Some(Ok(msg)) = futures_util::StreamExt::next(&mut stream).await {
                sink.on_message(&id, msg.payload_as_string());
            }
            tracing::debug!("Tuya 设备 {id} 事件流结束");
        });
    }
}

/// 控制回读/轮询共用的单次调用包装：外层再兜一层超时，防库内任务挂死拖住 API。
pub(crate) async fn with_call_timeout<F, T>(fut: F) -> Result<T, ControlFailure>
where
    F: Future<Output = Result<T, TuyaError>>,
{
    match tokio::time::timeout(CALL_TIMEOUT, fut).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(classify(&e)),
        Err(_) => Err(ControlFailure::new(
            "timeout",
            "设备调用超时（外层 6s 兜底）".into(),
            "稍后重试；若频发请检查设备所在 2.4G 网络",
        )),
    }
}

/// 在线/last_seen 更新（轮询与控制成功共用；失败只告警，不影响控制主流程）。
pub(crate) fn mark_online(store: &TuyaStore, device_id: &str, online: bool, ip: Option<&str>) {
    if let Err(e) = store.set_online(device_id, online, ip) {
        tracing::warn!("Tuya 设备 {device_id} 在线状态写库失败: {e}");
    }
}
