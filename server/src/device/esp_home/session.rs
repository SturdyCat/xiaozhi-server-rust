//! S3v：`[esp_home].engine = "wasm-session"` 的连接编排（生产接管）。
//!
//! ## 分工（协议面进插件，数据面留 native）
//!
//! - **插件（esphome.official）**：协议语义全部在 guest——`op=open` 产出 ServerHello
//!   （加密模式，psk 由插件**自身配置命名空间**的 `noise_psk` 提供，宿主不传）、
//!   `op=feed` 步进握手/消息派发/实体应答、`op=state` 推送状态帧、`op=close` 回收。
//!   事件间无内存态（状态在插件隔离 kv），连接结束/错误即回收。
//! - **宿主桥（本模块）**：TCP accept 后只搬字节——客户端字节**原样**喂 `op=feed`
//!   （不做任何协议解析）、`out_hex` 原样写回 socket；`state::subscribe()` 的变化
//!   组 `updates` 喂 `op=state`；从 feed 结果的 `messages` 里取 guest 解出的
//!   number 命令 → 复用 [`super::server::number_command_target`] +
//!   [`super::state::CatalogPlan`] 过滤 → [`super::actions::set_volume`]（与 native
//!   分支同一翻译缝）。BLE 代理不兼容该引擎（启动期守卫已拒绝）。
//!
//! ## 纪律
//!
//! - guest 调用是**阻塞**的（wasmtime 同步 API）：泵循环里一律
//!   `tokio::task::spawn_blocking` 隔离（weather/tuya 同纪律），不阻塞 reactor；
//! - fail-closed：open/feed 失败、插件未返回结果、加密认证失败（guest 侧已销毁
//!   状态）都直接断开连接，绝不降级；断开时 best-effort `op=close` 兜底回收；
//! - 会话 id 每连接唯一（`conn-<序号>-<对端端口>`，满足 guest 侧白名单
//!   `[A-Za-z0-9._-]`、长度 ≤64）；
//! - mDNS 的 `api_encryption` 在该引擎下**不宣告**：桥不读插件配置（凭证隔离，
//!   `config-secret` 读取只对插件自身审计），加密能力由 DeviceInfoResponse 的
//!   `api_encryption_supported` 如实上报（guest 依 psk 有无自填）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::broadcast::error::RecvError;

use crate::plugins::wasm::WasmHost;

use super::config::EspHomeConfig;
use super::{actions, state};

/// 会话事件所在的扩展点（`connector.esp-home` + `.session` topic）。
pub(crate) const EXTENSION_POINT: &str = "connector.esp-home";
/// 会话事件 topic。
const SESSION_TOPIC: &str = "connector.esp-home.session";
/// 承接会话事件的官方插件（启动期守卫按 id 钉死，防止探针类实现顶替）。
pub(crate) const SESSION_PLUGIN: &str = "esphome.official";
/// 结果通道前缀（与 guest `session_event` 同口径）。
const RESULT_PREFIX: &str = "esphome-session";
/// 会话状态 kv 前缀（回收断言/调试用）。
pub(crate) const STATE_PREFIX: &str = "esphome-session-state";
/// 单次读缓冲（客户端字节原样透传，大小只是搬运粒度；重组语义在 guest）。
const READ_CHUNK: usize = 4096;

/// 会话序号（每连接唯一 id 的单调部分）。
static SESSION_SEQ: AtomicU64 = AtomicU64::new(0);

// ============================================================
// 宿主 → 插件的一次会话事件步进
// ============================================================

/// 派发一次 `connector.esp-home.session` 事件并读回结果通道。
///
/// 阻塞 guest 调用经 `spawn_blocking` 隔离；结果写 `esphome-session:<req_id>`
/// （guest 侧效果），失败（宿主派发错误 / 效果执行失败 / 结果缺失或损坏）显式
/// `Err`——调用方按连接级失败处理，不静默降级。
async fn guest_call(host: &Arc<WasmHost>, mut payload: Value) -> Result<Value> {
    let req_id = uuid::Uuid::new_v4().to_string();
    payload["req_id"] = json!(req_id);
    let topic = SESSION_TOPIC.to_string();
    let payload = payload.to_string();
    let result_key = format!("{RESULT_PREFIX}:{req_id}");
    let host = Arc::clone(host);
    let bytes = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<u8>, String> {
        let errors = host
            .handle_event(SESSION_PLUGIN, &topic, &payload)
            .map_err(|e| format!("插件派发失败：{e:#}"))?;
        if !errors.is_empty() {
            return Err(format!("插件效果执行失败：{errors:?}"));
        }
        host.kv()
            .get(SESSION_PLUGIN, &result_key)
            .ok_or_else(|| format!("插件未返回结果（需要 kv 键 {result_key}）"))
    })
    .await
    .map_err(|e| anyhow::anyhow!("插件任务失败：{e}"))?
    .map_err(|e| anyhow::anyhow!(e))?;
    serde_json::from_slice(&bytes).map_err(|e| anyhow::anyhow!("插件结果损坏：{e}"))
}

/// 严格 hex 解码（与 guest 编码同口径）。
///
/// guest 的 `out_hex` 恒应合法；解码失败说明宿主/guest 边界出了严重问题，
/// 按连接级错误显式失败（fail-closed，不静默补零掩盖）。
fn hex_decode(hex: &str) -> Result<Vec<u8>> {
    if hex.len() % 2 != 0 {
        anyhow::bail!("hex 长度非法（奇数，{} 字符）", hex.len());
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        out.push(
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| anyhow::anyhow!("hex 非法：{e}"))?,
        );
    }
    Ok(out)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ============================================================
// open 载荷：身份 + 实体目录（放行过滤 + 当前快照）
// ============================================================

/// 设备身份（与 native 桥的 `EspHomeApi::builder` 同源；mac 用桥既有的稳定伪 MAC）。
///
/// BLE 位恒 false：BLE 消息面未进插件会话子集（守卫已拒绝 ble_proxy 组合，
/// 这里不谎报能力）。`api_encryption_supported` 由 guest 依 psk 有无自填。
fn identity_of(cfg: &EspHomeConfig) -> Value {
    json!({
        "name": cfg.device_name,
        "mac": super::server::fake_mac(&cfg.device_name),
        "esphome_version": "2026.6.2",
        "model": cfg.model,
        "manufacturer": cfg.manufacturer,
        "friendly_name": cfg.friendly_name,
        "suggested_area": cfg.suggested_area,
        "legacy_bluetooth_proxy_version": 0,
        "bluetooth_proxy_feature_flags": 0,
    })
}

/// 实体目录：`state::ENTITIES` × 目录计划——只含放行实体，显示名经计划覆盖，
/// **key 恒为静态索引**（状态回推与命令分派的 key 语义不变），状态取当前快照
/// （binary → bool、number → 数值）。
fn entities_of(plan: &state::CatalogPlan) -> Value {
    let binary = state::binary_snapshot();
    let list: Vec<Value> = state::ENTITIES
        .iter()
        .enumerate()
        .filter(|(key, _)| plan.is_exposed(*key))
        .map(|(key, spec)| match spec.kind {
            state::EntityKind::Binary { device_class } => json!({
                "key": key,
                "kind": "binary_sensor",
                "object_id": spec.object_id,
                "name": plan.display_name(key),
                "device_class": device_class,
                "state": binary.get(key).copied().unwrap_or(false),
            }),
            state::EntityKind::Number {
                min,
                max,
                step,
                unit,
                mode,
            } => json!({
                "key": key,
                "kind": "number",
                "object_id": spec.object_id,
                "name": plan.display_name(key),
                "min_value": min,
                "max_value": max,
                "step": step,
                "unit_of_measurement": unit,
                "mode": mode,
                "state": state::number_value(key),
            }),
        })
        .collect();
    json!(list)
}

/// 单实体的状态更新载荷（binary → bool、number → 数值；类型随实体种类）。
fn update_for(key: usize) -> Value {
    if key < state::BINARY_COUNT {
        let on = state::binary_snapshot().get(key).copied().unwrap_or(false);
        json!({"key": key, "state": on})
    } else {
        json!({"key": key, "state": state::number_value(key)})
    }
}

// ============================================================
// 命令执行（数据面留 native，语义与 native 分支一致）
// ============================================================

/// 测试观测替身：记录经 wasm-session 桥下达的音量档位（仅测试编译；
/// 生产路径的真正执行仍是 [`actions::set_volume`]）。
#[cfg(test)]
pub(crate) fn recorded_volume_commands() -> &'static std::sync::Mutex<Vec<u8>> {
    static REC: std::sync::OnceLock<std::sync::Mutex<Vec<u8>>> = std::sync::OnceLock::new();
    REC.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// 处理 guest 解出的命令消息：只有放行实体的 number 命令落到
/// [`actions::set_volume`]（命令**不回推**——设备会自己上报 volume_state，探针
/// 读真实值再推，提前推会把 HA 的滑块弹回旧值，与 native 分支同口径）。
fn execute_commands(plan: &state::CatalogPlan, messages: &Value) {
    let Some(list) = messages.as_array() else {
        return;
    };
    for msg in list {
        let Some(cmd) = msg.get("number_command") else {
            continue;
        };
        let key = cmd
            .get("key")
            .and_then(Value::as_u64)
            .and_then(|k| usize::try_from(k).ok())
            .unwrap_or(usize::MAX);
        // 目录计划未放行的实体不进 ListEntities，客户端不应发来命令；收到也只忽略。
        if !plan.is_exposed(key) {
            continue;
        }
        let state = cmd.get("state").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        if let Some(level) = super::server::number_command_target(key as u32, state) {
            tracing::debug!("[esp_home/session] 音量命令 {state:.2} → 档位 {level}");
            #[cfg(test)]
            recorded_volume_commands()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(level);
            actions::set_volume(level);
        }
    }
}

// ============================================================
// 单连接编排
// ============================================================

/// 单条 wasm-session 连接：open → 双向泵（feed / state）→ close。
///
/// 客户端字节**原样**进 `op=feed`（半包/粘包重组在 guest 流式缓冲）；
/// `out_hex` 原样写回（未订阅 / 无应答时为空，正常写 0 字节）。
pub(crate) async fn handle_conn(
    stream: TcpStream,
    cfg: &EspHomeConfig,
    host: Arc<WasmHost>,
    plan: state::CatalogPlan,
) -> Result<()> {
    let peer = stream.peer_addr().ok();
    let session = format!(
        "conn-{}-{}",
        SESSION_SEQ.fetch_add(1, Ordering::Relaxed),
        peer.map(|a| a.port()).unwrap_or(0)
    );
    let (mut reader, mut writer) = stream.into_split();

    // open + 双向泵；无论从哪条路径退出（含写回失败 / 派发失败），最后都
    // best-effort `op=close` 回收会话状态——kv 里的会话是有限资源（MAX_SESSIONS），
    // 不能因为任何一条早退路径泄漏。
    let result = pump(&host, &mut reader, &mut writer, cfg, &plan, &session).await;

    // op=close：回收会话状态（best-effort 兜底；feed 失败路径 guest 已自行销毁）。
    let _ = guest_call(&host, json!({"op": "close", "session": session})).await;
    tracing::debug!("[esp_home/session] 会话 {session} 已关闭（peer={peer:?}）");
    result
}

/// open + 双向泵（见 [`handle_conn`]；抽出以便任何退出路径都经过 close 回收）。
async fn pump(
    host: &Arc<WasmHost>,
    reader: &mut tokio::net::tcp::OwnedReadHalf,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    cfg: &EspHomeConfig,
    plan: &state::CatalogPlan,
    session: &str,
) -> Result<()> {
    // op=open：加密模式（插件配置了 noise_psk）产出 ServerHello，写回 socket；
    // 明文模式 out 为空。失败（含 psk 配置非法——guest fail-closed）直接断开。
    let open = guest_call(
        host,
        json!({
            "op": "open",
            "session": session,
            "identity": identity_of(cfg),
            "entities": entities_of(plan),
        }),
    )
    .await?;
    if open["ok"] != json!(true) {
        let error = open["error"].as_str().unwrap_or("未知").to_string();
        anyhow::bail!("会话建立失败：{error}");
    }
    let out = hex_decode(open["out_hex"].as_str().unwrap_or_default())?;
    if !out.is_empty() {
        writer.write_all(&out).await?;
    }
    tracing::debug!(
        "[esp_home/session] 会话 {session} 已打开（mode={}，peer={:?}）",
        open["mode"].as_str().unwrap_or("?"),
        reader.as_ref().peer_addr().ok(),
    );
    // 订阅既有真相源的状态广播；每条变化组单实体 updates 喂 op=state。
    let mut changes = state::subscribe();
    let mut chunk = vec![0u8; READ_CHUNK];
    loop {
        tokio::select! {
            read = reader.read(&mut chunk) => {
                match read {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let feed = guest_call(
                            host,
                            json!({
                                "op": "feed",
                                "session": session,
                                "data_hex": hex_encode(&chunk[..n]),
                            }),
                        )
                        .await;
                        let feed = match feed {
                            Ok(v) if v["ok"] == json!(true) => v,
                            Ok(v) => {
                                // 协议/认证失败：guest 已 fail-closed 销毁状态，
                                // 不降级、不重试，直接断开。
                                let error = v["error"].as_str().unwrap_or("未知").to_string();
                                anyhow::bail!("会话步进失败：{error}");
                            }
                            Err(e) => return Err(e),
                        };
                        let out = hex_decode(feed["out_hex"].as_str().unwrap_or_default())?;
                        if !out.is_empty() {
                            writer.write_all(&out).await?;
                        }
                        execute_commands(plan, &feed["messages"]);
                        // 对端发了 DisconnectRequest：应答已写回，正常收尾。
                        if feed["closed"] == json!(true) {
                            break;
                        }
                    }
                }
            }
            change = changes.recv() => {
                match change {
                    Ok(key) => {
                        if !plan.is_exposed(key) {
                            continue;
                        }
                        push_state(host, writer, session, &[update_for(key)]).await?;
                    }
                    // 丢事件就全量补一次（仅放行实体），保证客户端最终一致。
                    Err(RecvError::Lagged(_)) => {
                        let updates: Vec<Value> = (0..state::ENTITIES.len())
                            .filter(|key| plan.is_exposed(*key))
                            .map(update_for)
                            .collect();
                        push_state(host, writer, session, &updates).await?;
                    }
                    Err(RecvError::Closed) => {}
                }
            }
        }
    }
    Ok(())
}

/// 推一次状态（订阅态由插件管理：未订阅时 out 为空，正常写 0 字节）。
///
/// 插件侧业务失败（ok:false）/派发失败只告警不断连：单次状态推送失败不影响协议
/// 会话（下一轮变化会再推）。**guest `out_hex` 非法是边界完整性问题**，按连接级
/// 错误返回（与 open/feed 的严格 hex 口径一致，fail-closed 不静默）。
async fn push_state(
    host: &Arc<WasmHost>,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    session: &str,
    updates: &[Value],
) -> Result<()> {
    let result = guest_call(
        host,
        json!({"op": "state", "session": session, "updates": updates}),
    )
    .await;
    match result {
        Ok(v) if v["ok"] == json!(true) => {
            let out = hex_decode(v["out_hex"].as_str().unwrap_or_default())?;
            if !out.is_empty() {
                writer.write_all(&out).await?;
            }
        }
        Ok(v) => {
            tracing::warn!(
                "[esp_home/session] 状态推送被拒绝：{}",
                v["error"].as_str().unwrap_or("未知")
            );
        }
        Err(e) => tracing::warn!("[esp_home/session] 状态推送派发失败：{e:#}"),
    }
    Ok(())
}
