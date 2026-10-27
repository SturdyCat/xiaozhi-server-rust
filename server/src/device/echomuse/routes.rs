//! `/em/control` 与 `/em/data` 的 axum 路由与两条 inbound 任务。
//!
//! 生命周期：control 连接 → register 验证 → `ack`/`config` 下发 → 登记出站口 →
//! 起 control 任务 → 进入 [`run_session`]（EchoMuse 会话**无 xiaozhi hello 交换**，
//! 会话参数由 [`super::session_params`] 固定构造）。data 连接随后到达，按
//! `?device_id=`（显式）或"当前唯一待认领设备"认领出站口。
//!
//! 通道接线：control 会话建三条 channel——上行（翻译后的会话帧 → [`EchomuseTransport`]）、
//! control 出站（mic_start 等）、data 出站（`0x02`/`0x03`）。hub 登记的
//! [`PendingDevice`] 持有上行发送端与 data 接收端，data 路由认领后由 data 任务使用。
//!
//! 任务退出：control 任务收到 [`ControlOut::Shutdown`] 或对端断开即退出并清登记；
//! data 任务发现对端断开即退出。任一退出都会让上行 channel 关闭，会话侧
//! `recv` 报 `Closed`，整条会话随之收尾（与 WsTransport 的粘滞 `Closed` 同语义）。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use std::net::{IpAddr, SocketAddr};
use tokio::sync::mpsc;

use crate::runtime::Engines;
use crate::workflow::protocol::IncomingFrame;
use crate::workflow::session::run_session;

use super::ble::BleLink;
use super::transport::EchomuseTransport;
use super::uplink::MicUplink;
use super::{
    ack_message, config_baseline, config_push_with, hub_claim_data, hub_claim_only, hub_insert,
    hub_restore_data, session_params, ControlOut, DataOut, DeviceRegister,
};

/// data 面入站消息认领失败时的日志前缀（诊断"只连了 data 没 control"类配置错误）。
const CLAIM_FAIL: &str = "[echomuse] /em/data 无待认领设备（control 未就绪、已认领或多台设备并存）";

/// 等首帧 identify 的上限：真机固件拨通立即发；无 identify 的客户端（旧 mock）
/// 超时后按"唯一待认领设备"的旧规则兜底，不无限挂住连接。
const IDENTIFY_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// `/data` 链路空闲保活间隔：官方 controller 在同一条 data WS 上周期性发协议层
/// WS Ping（见 `tools/echomuse_mock_device.py` 的 `recv_message`：`0x9 → 0xA`）。
/// 缺了它，链路长时间无字节（监听中设备语音门未开时尤其如此）会被设备侧读超时或
/// 中间设备以 `close 1006` 踢掉——真机日志里每 ~15-20s 重拨一次就是这个。
/// 15s 一拍，短于常见 30s 空闲阈值，也不至于刷屏。
const DATA_PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

/// 挂载 EchoMuse 路由（`[echomuse].enabled=true` 时由 [`crate::gateway::ws::router`] 调）。
pub fn em_routes() -> axum::Router<Arc<Engines>> {
    use axum::routing::get;
    axum::Router::new()
        // 上游固件（现版）拨的是根路径 `/control`、`/data`；`/em/*` 是本项目
        // 旧版接口文档的挂载名，保留作别名（mock 脚本与文档走它）。
        .route("/control", get(em_control))
        .route("/data", get(em_data))
        .route("/em/control", get(em_control))
        .route("/em/data", get(em_data))
        .route("/shell/{id}", get(super::shell::upgrade))
        // 只读状态查询（排查"设备是否连上服务器"）：关闭时也返回 200 + enabled=false，
        // 不像 WS 路由那样 404——该接口本身就是用来区分这两种情况的。
        .route("/em/status", get(em_status))
}

/// `GET /em/status`：`{"enabled":bool,"devices":[{device_id,connected_secs}]}`。
/// devices 取 hub 当前 key 集合（= 处于会话中的在线设备）；无鉴权（不含敏感信息）。
async fn em_status(State(engines): State<Arc<Engines>>) -> Response {
    axum::Json(super::status_json(engines.config.echomuse.enabled)).into_response()
}

/// `X-Em-Token` 校验：配置为空则不校验（与 `[server].expected_token` 同口径）。
fn token_ok(headers: &HeaderMap, expected: &str) -> bool {
    if expected.is_empty() {
        return true;
    }
    headers.get("X-Em-Token").and_then(|v| v.to_str().ok()) == Some(expected)
}

async fn em_control(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(engines): State<Arc<Engines>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    let cfg = &engines.config.echomuse;
    if !cfg.enabled {
        return (StatusCode::NOT_FOUND, "echomuse disabled").into_response();
    }
    // token 不匹配时不立即 401：上游固件的配对流程（装过 CA 的设备长按 action 键
    // 降级 plain 拨号）**不带 token**，是否放行要等 register 里的 `pairing:true`
    // ——升级后由 control_session 首帧裁决，token 错误的普通连接照样拒绝。
    let authenticated = token_ok(&headers, &cfg.token);
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = control_session(socket, engines, authenticated, Some(peer.ip())).await {
            tracing::warn!("[echomuse] control 会话异常: {e}");
        }
    })
}

/// control 主体：register → ack → 起 control 任务 → 进会话主循环。
async fn control_session(
    socket: WebSocket,
    engines: Arc<Engines>,
    authenticated: bool,
    peer: Option<IpAddr>,
) -> anyhow::Result<()> {
    let (mut sink, mut stream) = socket.split();
    // 第一条消息必须是 register（协议规定"连接后立刻发送"）。
    let register: DeviceRegister = match stream.next().await {
        Some(Ok(Message::Text(t))) => {
            serde_json::from_str(&t).map_err(|e| anyhow::anyhow!("register 解析失败: {e}"))?
        }
        Some(Ok(_)) => anyhow::bail!("首条消息不是文本 register"),
        _ => anyhow::bail!("连接在 register 前关闭"),
    };
    if register.device_id.is_empty() {
        anyhow::bail!("register 缺 device_id");
    }
    let device_id = register.device_id.clone();
    tracing::info!(
        "[echomuse] 设备接入：device={device_id} ver={} board={:?} os={:?} caps={} ip={:?} kernel={:?}/{:?} boot={:?}/{:?}",
        register.version,
        register.board,
        register.base_os,
        register.capabilities.join(","),
        register.ip,
        register.kernel_arch,
        register.kernel_release,
        register.boot_id,
        register.boot_reason,
    );

    // 准入裁决（官方语义：`refused` = 凭据不对；`pending` = 未 admit，保持重拨；
    // 见 docs/device-controller-interface.md §/control Controller→Device）。
    // 固件收到 refused 会显示拒绝环（提示长按 action 配对），收到 pending 继续
    // 每 5s/2min 重拨；老固件两者都忽略，行为等同直接断链（与旧实现一致）。
    let registry = super::registry::global(&engines.config);
    if !authenticated {
        if !register.pairing {
            // token 缺失/错误：先告诉设备"为什么"，再断。
            let _ = sink
                .send(Message::Text(r#"{"type":"refused"}"#.into()))
                .await;
            anyhow::bail!("token 缺失/错误，拒绝接入");
        }
        // 配对窗口：登记（管理台可见、可批）+ pending，不建会话；批准后重拨即接入。
        registry.upsert(&register, super::registry::now_secs());
        super::bump_devices_version();
        let _ = sink
            .send(Message::Text(r#"{"type":"pending","pairing":true}"#.into()))
            .await;
        tracing::warn!(
            "[echomuse] 配对请求：device={device_id}（无 token），已登记待审批，本连接不建立"
        );
        return Ok(());
    }
    if !registry
        .get(&device_id)
        .map(|r| r.approved)
        .unwrap_or(false)
    {
        // token 对但未审批：同 pending 语义——登记 + 等管理台批准，不建会话。
        // （此前审批从未真正生效：register 即建会话。首台设备在管理台批准一次即可。）
        registry.upsert(&register, super::registry::now_secs());
        super::bump_devices_version();
        let _ = sink
            .send(Message::Text(r#"{"type":"pending"}"#.into()))
            .await;
        tracing::info!(
            "[echomuse] device={device_id} 已认证但未审批：登记 + pending，等待管理台批准"
        );
        return Ok(());
    }

    // 蓝牙代理链路按协议双向协商：服务器配置开启 且 设备声明 ble_connect 才在 ack 里
    // 回声明（未声明即降级，设备侧扫描/连接保持关闭）。
    let ble_on = engines.config.echomuse.ble_proxy
        && register.capabilities.iter().any(|c| c == "ble_connect");
    // ack（能力协商：只声明服务器支持的子集）+ config 推送（基线 + 每设备保存的设置）。
    sink.send(Message::Text(ack_message(&device_id, ble_on).into()))
        .await?;
    let settings = super::registry::global(&engines.config)
        .get(&device_id)
        .map(|r| serde_json::Value::Object(r.settings.into_iter().collect()))
        .unwrap_or_else(|| serde_json::json!({}));
    for msg in config_push_with(&config_baseline(
        engines.config.echomuse.ble_proxy,
        &settings,
    )) {
        sink.send(Message::Text(msg.into())).await?;
    }

    // 三条 channel：上行（翻译后的会话帧）、control 出站、data 出站。
    let (up_tx, up_rx) = mpsc::unbounded_channel::<IncomingFrame>();
    let (ctl_tx, ctl_rx) = mpsc::unbounded_channel::<ControlOut>();
    let (data_tx, data_rx) = mpsc::unbounded_channel::<DataOut>();

    // 静态元数据落盘（label/approved/版本/首末见时间；online 由 hub 实时合并）。
    super::registry::global(&engines.config).upsert(&register, super::registry::now_secs());

    let ble = Arc::new(std::sync::Mutex::new(BleLink::default()));
    let live = Arc::new(super::runtime::LiveControl::new(peer, authenticated));
    // 采集指令与灯效/提示音绑定本会话的 control 出站（同一条 FIFO，保序）。
    // 灯效能力随 register 的 capabilities 传入（官方 `led_anim_capable`：
    // 新固件本地渲染动画，旧固件退回静态 leds 帧）。
    live.mic.bind(ctl_tx.clone());
    live.fx.bind_with_caps(
        ctl_tx.clone(),
        register.capabilities.iter().any(|c| c == "led_anim"),
    );
    hub_insert(
        &device_id,
        super::PendingDevice {
            up_tx: up_tx.clone(),
            data_rx: Some(data_rx),
            since: std::time::Instant::now(),
            // 管理面下发 volume_set/shell_open/wifi_change；control 会话自己持原件。
            ctl_tx: Some(ctl_tx.clone()),
            // 蓝牙代理 GATT 请求（0x08）的管理面下发口；control 会话自己持原件。
            data_tx: Some(data_tx.clone()),
            ble: ble.clone(),
            management: live.clone(),
        },
    );

    // 设备侧 OWW 资产自动下发：对齐官方 em_oww_sync——语音门模型 + ORT 运行时**自动**
    // 同步到设备，不靠手动按钮。仅在已批准、在线、emOS 设备接入时触发；幂等（设备侧
    // sha256 已匹配则跳过，避免每次重连重传）。资产未就绪时静默跳过。
    super::oww::ensure_on_device_assets(device_id.clone(), register.base_os.clone());

    // control 任务：收设备事件（唤醒/按钮/旧固件 ble_adverts）+ 发 mic_start 等出站。
    // 常开唤醒流（官方接入后立即下发）：omni ch6、无门控。波束基线/AEC/影子评分
    // 的持续输入；缺了它，首个 turn 流刚起就 beam.Lock() 必落 `baseline not ready`。
    // 指令进 FIFO channel，control_loop spawn 后按序发出。
    live.mic.wake_stream();
    let media = live.media.clone();
    let lease = super::runtime::ControlLease {
        id: device_id.clone(),
        ctl: ctl_tx.clone(),
        live,
    };
    let ctl_task = tokio::spawn(control_loop(
        sink, stream, ctl_rx, up_tx, ble, ble_on, lease,
    ));

    // 进入会话主循环（无 hello：参数固定，设备不做 xiaozhi 协商；kind=EchoMuse 由
    // `session_params` 的 PcmS16Le⇔EchoMuse 口径定型，见 `configured_defaults`）。
    let params = session_params(&engines.config);
    // 连接欢迎语由本入口决策并注入会话（`[echomuse].welcome`，RestartOnly）：
    // workflow 只收文本，不反向依赖 device::echomuse 的配置。
    let welcome = super::connect_welcome("echomuse", &engines.config.echomuse);
    let session_id = uuid::Uuid::new_v4().to_string();
    // 几何上日志：排查设备端语速/音调问题的第一现场（协议常量，非协商值）。
    tracing::info!(
        "session {session_id} [echomuse] {device_id} 会话几何：下行 {}Hz/{}ms mono PCM（{}B/帧，设备 DAC 直出不重采样） 上行 16k/40ms",
        params.output_format().sample_rate,
        params.downlink_frame_ms,
        super::frames::SPEAKER_CHUNK_BYTES
    );
    let result = run_session(
        &mut EchomuseTransport::with_media(up_rx, ctl_tx, data_tx, media),
        engines,
        params,
        session_id,
        device_id.clone(),
        "echomuse".to_string(),
        false,
        false,
        welcome,
    )
    .await;

    // 收尾：停 control 任务、清登记（会话 close 已发 Shutdown，这里是兜底）。
    ctl_task.abort();
    // Wait for lease Drop; an old session cannot delete a replacement register.
    let _ = ctl_task.await;
    result
}

/// control 任务：设备事件 → 会话语义；出站指令 → JSON 下发。
async fn control_loop(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut stream: futures_util::stream::SplitStream<WebSocket>,
    mut ctl_rx: mpsc::UnboundedReceiver<ControlOut>,
    up_tx: mpsc::UnboundedSender<IncomingFrame>,
    ble: Arc<std::sync::Mutex<BleLink>>,
    ble_on: bool,
    lease: super::runtime::ControlLease,
) {
    loop {
        tokio::select! {
            inbound = stream.next() => {
                let Some(Ok(msg)) = inbound else { break };
                if let Message::Text(t) = msg {
                    if t.len() <= super::runtime::MAX_TELEMETRY_BYTES {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) { lease.live.update(&v); }
                    }
                    handle_control_text(&t, &lease.id, &lease.live, &up_tx, &ble, ble_on);
                }
            }
            out = ctl_rx.recv() => {
                match out {
                    Some(ControlOut::Json(json)) => {
                        if sink.send(Message::Text(json.into())).await.is_err() { break; }
                    }
                    Some(ControlOut::Shutdown) | None => break,
                }
            }
        }
    }
}

/// 解析 control 面的一条设备 JSON。只关心能开启一轮聆听的事件，其余忽略
/// （未知类型双向忽略；listen_state/stats/volume_state/mute_state 已由 control_loop
/// 先写入有界 runtime，其余 log/pong 等仍忽略）。
/// `ble_adverts` 是旧固件的广播回退路径（`0x06` 未被 ack 声明时设备改走 control 面）。
/// `device_id` 只用于日志（唤醒/按键是用户最关心的"收到了什么事件"，行里要能对上设备）。
/// 开轮走 [`super::mic::MicControl`]（官方三连时序），采集收口在 data 侧
/// [`super::uplink::MicUplink`] 的 0x04/0x05 路径。
fn handle_control_text(
    text: &str,
    device_id: &str,
    live: &super::runtime::LiveControl,
    up_tx: &mpsc::UnboundedSender<IncomingFrame>,
    ble: &Arc<std::sync::Mutex<BleLink>>,
    ble_on: bool,
) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return; // control 面恒为 JSON；防御式忽略而非报错
    };
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match kind {
        // 设备本地唤醒词命中（oww_trigger）。服务器未声明 `listen_session`，设备不会走
        // 私听会话（无 session 字段），这里直接开 bounded turn。
        // 来源标签随 detect 的 text 透传给会话层（会话日志据此区分唤醒词 vs 按键）。
        "oww_wake" => {
            tracing::info!("[echomuse] device {device_id} 触发来源：本地唤醒词，开轮");
            super::turn::start_turn(live, up_tx, "本地唤醒词", true)
        }
        // Dot2 专属适配：只有 action 键（clickType=138 / button.type=="action"）才开轮；
        // 音量± / 禁麦由设备本地处理（ADC 静音 / 调音量，分别上报 mute_state / volume_state），
        // 控制器不应为其开语音轮。
        //
        // **判定时机逐字对齐官方** `em_controller.handle_button_event`：
        //   `if down: return`               —— 按下什么都不做，判定全在"松开"时（heldMs 才是真值）
        //   held_ms >= BUTTON_HOLD_MS(750)  —— HOLD：官方转成 Home Assistant 的 long 事件，不开轮
        //   muted                           —— BLOCKED
        //   turn_active (voice_lock)        —— **CANCEL**：再点一下是取消，不是忽略
        //   其余                            —— TURN
        // 其中 CANCEL 是自愈的关键：此前这里对 turn_active 静默丢弃，导致一轮只要没收到
        // 设备 0x04/0x05 就永久占住采集门，之后每次按键全部无效（现场 12:32→13:04 复现）。
        "button" => {
            if !super::dot2::is_action_button(&v) {
                let kind = v
                    .get("clickType")
                    .and_then(Value::as_i64)
                    .map(super::dot2::Dot2Button::from_click_type)
                    .unwrap_or(super::dot2::Dot2Button::Other);
                tracing::debug!(
                    "[echomuse] device {device_id} 非 action 按键（{:?}, clickType={:?}），不触发语音轮",
                    kind,
                    v.get("clickType")
                );
            } else {
                let down = v.get("down").and_then(Value::as_bool).unwrap_or(true);
                let held_ms = v.get("heldMs").and_then(Value::as_i64).unwrap_or(0);
                if down {
                    // 官方同款：按下直接返回，不给任何反馈。
                } else if held_ms >= super::dot2::BUTTON_HOLD_MS {
                    tracing::debug!(
                        "[echomuse] device {device_id} action 键长按 {held_ms}ms → 官方语义为 HA long 事件，不开轮"
                    );
                } else if live.is_muted() {
                    tracing::debug!(
                        "[echomuse] device {device_id} 静音状态下忽略 action 键（官方 BLOCKED；设备侧也会拒绝 mic_start）"
                    );
                } else if live.turn.is_active() {
                    tracing::info!(
                        "[echomuse] device {device_id} action 键再按 → 取消当前轮（官方 CANCEL）"
                    );
                    super::turn::cancel_turn(live, up_tx)
                } else {
                    // 手动按键 ≠ 唤醒词命中：对齐官方 `_button_voice_turn` 的
                    // `is_wakeword=False`——直接进聆听（亮聆听环）、不播唤醒问候「我在」、
                    // 不播唤醒提示音。用户按这个键就是"我要说话"。
                    tracing::info!("[echomuse] device {device_id} 触发来源：action 键，开轮");
                    super::turn::start_turn(live, up_tx, "action 键", false)
                }
            }
        }
        "ble_adverts" if ble_on => {
            if let Some(list) = v.get("adverts").and_then(|a| a.as_array()) {
                ble.lock()
                    .expect("echomuse ble link 锁中毒")
                    .merge_adverts(list, super::registry::now_secs());
            }
        }
        // 设备日志行：透传进服务端日志（官方还持久化到 dashboard/支持包，
        // 这里达到"不用插线也能看到设备侧 sh: syntax error 之类"的目的即可）。
        "log" => {
            let level = v.get("level").and_then(|l| l.as_str()).unwrap_or("info");
            let message = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
            if !message.is_empty() {
                let line = format!("[echomuse] {device_id} 设备日志: {message}");
                match level {
                    "error" => tracing::error!("{line}"),
                    "warn" | "warning" => tracing::warn!("{line}"),
                    "debug" => tracing::debug!("{line}"),
                    _ => tracing::info!("{line}"),
                }
            }
        }
        _ => {}
    }
}

#[path = "routes_tests.rs"]
#[cfg(test)]
mod tests;

async fn em_data(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    let cfg = &engines.config.echomuse;
    if !cfg.enabled {
        return (StatusCode::NOT_FOUND, "echomuse disabled").into_response();
    }
    if !token_ok(&headers, &cfg.token) {
        return (StatusCode::UNAUTHORIZED, "bad token").into_response();
    }
    ws.on_upgrade(move |socket| async move {
        // 首帧认领：真机固件拨 /data 后**先发一条 Text identify**
        // `{"type":"identify","device_id":…}`（上游 device/internal/client/data.go
        // WriteMessage(TextMessage, …)——data 面唯一约定的文本帧），凭它精确认领，
        // 多台设备并存也能对号入座；mock/测试路径没有 identify，沿用 ?device_id=
        // 显式或"唯一待认领设备"的旧规则。首帧若是二进制（旧客户端开麦即推），
        // 原样作为 prelude 交给 data_loop，不丢首包。
        let explicit = query.get("device_id").cloned();
        let mut socket = socket;
        let mut identify: Option<String> = explicit;
        let mut prelude: Option<Message> = None;
        if identify.is_none() {
            match tokio::time::timeout(IDENTIFY_WAIT, socket.recv()).await {
                Ok(Some(Ok(Message::Text(t)))) => identify = parse_identify(&t),
                Ok(Some(Ok(frame))) => prelude = Some(frame),
                _ => {}
            }
        }
        let claimed = match identify.as_deref() {
            Some(id) => hub_claim_data(id),
            None => hub_claim_only(),
        };
        let Some(claim) = claimed else {
            tracing::warn!("{CLAIM_FAIL}");
            return;
        };
        let ble_on = engines.config.echomuse.ble_proxy;
        // data 任务退出时归还认领：链路闪断重拨立即可再认领，不必等下次 register。
        if let Some(claim) = data_loop(socket, claim, ble_on, prelude).await {
            hub_restore_data(&claim.device_id, claim.data_rx, &claim.data_tx);
        }
    })
}

/// 解析 data 面首帧的 Text identify（真机固件拨通后立即发送；mock/测试可省略）。
/// 返回 device_id；形状不符（非 identify、缺/空 device_id、非法 JSON）返回 None，
/// 由调用方按旧规则（唯一待认领设备）兜底。`pub(super)` 供外置测试直驱。
pub(super) fn parse_identify(text: &str) -> Option<String> {
    let v = serde_json::from_str::<serde_json::Value>(text).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("identify") {
        return None;
    }
    let id = v.get("device_id").and_then(|d| d.as_str())?;
    (!id.is_empty()).then(|| id.to_string())
}

/// data 任务：上行帧翻译（PCM→Opus、typed utterance end、BLE 0x06/0x08 采集）
/// + 下行帧直发。`prelude` 是认领前替读到的首帧（无 identify 客户端的先行二进制），
/// 认领后补处理。返回 `Some(claim)` 表示应把认领归还 hub（所有退出路径都归还）。
async fn data_loop(
    socket: WebSocket,
    claim: super::DataClaim,
    ble_on: bool,
    prelude: Option<Message>,
) -> Option<super::DataClaim> {
    let super::DataClaim {
        device_id,
        up_tx,
        mut data_rx,
        data_tx,
        ble,
        management,
    } = claim;
    let (mut sink, mut stream) = socket.split();
    let mut uplink = MicUplink::for_media(
        management.media.clone(),
        management.mic.clone(),
        management.fx.clone(),
    );
    // 认领前替读的首帧（若有）按正常上行处理，不丢首包。
    if let Some(Message::Binary(raw)) = prelude {
        uplink.on_binary(&raw, &management.turn, &up_tx, &ble, ble_on);
    }
    // 周期 Ping 保活（对齐官方 controller）。interval 首拍立即就绪，先消费掉，
    // 避免建链瞬间抢在下行首帧之前发一帧。
    let mut ping = tokio::time::interval(DATA_PING_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.tick().await;
    loop {
        tokio::select! {
            // 保活：写失败说明对端已断，退出。
            _ = ping.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            inbound = stream.next() => {
                match inbound {
                    // 协议：/data 设备方向除首帧 Text identify 外恒为二进制；此后再收到
                    // Text（identify 重发等固件行为差异）一律忽略——绝不能断链，此前在
                    // 此把 Text 当致命帧 break，真机固件的 identify 直接把 data 链踢掉
                    // （设备侧表现为 close 1006 每 5s 重连）。
                    Some(Ok(Message::Binary(raw))) => {
                        uplink.on_binary(&raw, &management.turn, &up_tx, &ble, ble_on);
                    }
                    // 心跳/控制帧不是断链信号：Ping 由 axum 自动回 Pong，Pong 是设备
                    // 回我们的心跳。此前 `_ => break` 会把设备对我们 Ping 的那次 Pong
                    // 当作致命帧——等于自己发的心跳把自己踢掉，正是 1006 循环的成因。
                    Some(Ok(Message::Text(_)))
                    | Some(Ok(Message::Ping(_)))
                    | Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) => break,
                    // 读错误 / 流结束：真断链。
                    _ => break,
                }
            }
            out = data_rx.recv() => {
                match out {
                    Some(frame) => {
                        let written = super::data_writer::write_frame(&mut sink, &mut stream, frame, &management.media,
                            |raw| uplink.on_binary(raw, &management.turn, &up_tx, &ble, ble_on)).await;
                        if !written { break; }
                    }
                    None => break,
                }
            }
        }
    }
    // The caller returns this claim through hub_restore_data. That function
    // atomically resets BLE, drains old outbound frames, and makes data claimable;
    // resetting separately here would race a new GATT request or data claimant.
    Some(super::DataClaim {
        device_id,
        up_tx,
        data_rx,
        data_tx,
        ble,
        management,
    })
}
