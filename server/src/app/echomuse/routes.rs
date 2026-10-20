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

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use std::net::{IpAddr, SocketAddr};
use tokio::sync::mpsc;

use crate::app::audio::opus::OpusFrameEncoder;
use crate::app::session::run_session;
use crate::app::transport::IncomingFrame;
use crate::engine::Engines;

use super::ble::BleLink;
use super::frames::{self, UplinkFrame};
use super::transport::EchomuseTransport;
use super::{
    ack_message, config_baseline, config_push_with, hub_claim_data, hub_claim_only, hub_insert,
    hub_restore_data, session_params, ControlOut, DataOut, DeviceRegister,
};

/// data 面入站消息认领失败时的日志前缀（诊断"只连了 data 没 control"类配置错误）。
const CLAIM_FAIL: &str = "[echomuse] /em/data 无待认领设备（control 未就绪、已认领或多台设备并存）";

/// 等首帧 identify 的上限：真机固件拨通立即发；无 identify 的客户端（旧 mock）
/// 超时后按"唯一待认领设备"的旧规则兜底，不无限挂住连接。
const IDENTIFY_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// 挂载 EchoMuse 路由（`[echomuse].enabled=true` 时由 [`crate::app::ws::router`] 调）。
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
    // 鉴权裁决（见 em_control）：无 token 且不是配对请求 → 拒绝；配对请求放行
    // 但明确告警，设备是否可用仍由管理面审批（registry.approved）决定。
    if !authenticated {
        if !register.pairing {
            anyhow::bail!("token 缺失/错误，拒绝接入");
        }
        tracing::warn!(
            "[echomuse] 配对请求：device={}（无 token，请到管理台认领/审批）",
            register.device_id
        );
    }
    let device_id = register.device_id.clone();
    tracing::info!(
        "[echomuse] 设备接入：device={device_id} ver={} board={:?} os={:?} caps={}",
        register.version,
        register.board,
        register.base_os,
        register.capabilities.join(","),
    );

    // 蓝牙代理按协议双向协商：服务器配置开启 且 设备声明 ble_connect 才在 ack 里
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

    // control 任务：收设备事件（唤醒/按钮/旧固件 ble_adverts）+ 发 mic_start 等出站。
    let lease = super::runtime::ControlLease {
        id: device_id.clone(),
        ctl: ctl_tx.clone(),
        live,
    };
    let ctl_task = tokio::spawn(control_loop(
        sink, stream, ctl_rx, up_tx, ble, ble_on, lease,
    ));

    // 进入会话主循环（无 hello：参数固定，设备不做 xiaozhi 协商）。
    let params = session_params(&engines.config);
    let session_id = uuid::Uuid::new_v4().to_string();
    let result = run_session(
        &mut EchomuseTransport::new(up_rx, ctl_tx, data_tx)?,
        engines,
        params,
        session_id,
        device_id.clone(),
        "echomuse".to_string(),
        false,
        false,
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
    // 当前是否处于一轮聆听（唤醒后到哨兵/停止之间）。
    let mut listening = false;
    loop {
        tokio::select! {
            inbound = stream.next() => {
                let Some(Ok(msg)) = inbound else { break };
                if let Message::Text(t) = msg {
                    if t.len() <= super::runtime::MAX_TELEMETRY_BYTES {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) { lease.live.update(&v); }
                    }
                    if handle_control_text(&t, &mut listening, &mut sink, &up_tx, &ble, ble_on)
                        .await
                        .is_err()
                    {
                        break;
                    }
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
async fn handle_control_text(
    text: &str,
    listening: &mut bool,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    up_tx: &mpsc::UnboundedSender<IncomingFrame>,
    ble: &Arc<std::sync::Mutex<BleLink>>,
    ble_on: bool,
) -> anyhow::Result<()> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Ok(()); // control 面恒为 JSON；防御式忽略而非报错
    };
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match kind {
        // 设备本地唤醒词命中（oww_trigger）。服务器未声明 `listen_session`，设备不会走
        // 私听会话（无 session 字段），这里直接开 bounded turn（mic_start + listen start）。
        "oww_wake" => start_turn(listening, sink, up_tx).await?,
        // 动作键单击 = 无唤醒词模型的兜底入口；mute/音量走 mute_state/volume_state，不进这里。
        "button" => start_turn(listening, sink, up_tx).await?,
        "ble_adverts" if ble_on => {
            if let Some(list) = v.get("adverts").and_then(|a| a.as_array()) {
                ble.lock()
                    .expect("echomuse ble link 锁中毒")
                    .merge_adverts(list, super::registry::now_secs());
            }
        }
        _ => {}
    }
    Ok(())
}

/// 开一轮聆听：`mic_start{lock_mic:true}`（设备做 VAD 的 bounded turn，结束发
/// `0x04`/`0x05` 哨兵）+ 会话侧 `listen state=start`。
async fn start_turn(
    listening: &mut bool,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    up_tx: &mpsc::UnboundedSender<IncomingFrame>,
) -> anyhow::Result<()> {
    if *listening {
        return Ok(()); // 一轮已在进行，忽略重复唤醒
    }
    *listening = true;
    sink.send(Message::Text(
        r#"{"type":"mic_start","lock_mic":true}"#.into(),
    ))
    .await?;
    let _ = up_tx.send(IncomingFrame::Text(
        r#"{"type":"listen","state":"start"}"#.into(),
    ));
    Ok(())
}

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

/// data 任务：上行帧翻译（PCM→Opus、哨兵→listen stop、蓝牙代理 0x06/0x08 采集）
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
    } = claim;
    let (mut sink, mut stream) = socket.split();
    let mut up_enc = match OpusFrameEncoder::new(frames::MIC_SAMPLE_RATE) {
        Ok(enc) => enc,
        Err(e) => {
            tracing::warn!("[echomuse] 上行编码器创建失败: {e}");
            return Some(super::DataClaim {
                device_id,
                up_tx,
                data_rx,
                data_tx,
                ble,
            });
        }
    };
    // 上行 PCM 暂存：攒满一个 60ms 帧（960 样本）编一包 Opus。
    let mut up_pcm: Vec<i16> = Vec::with_capacity(frames::MIC_FRAME_SAMPLES * 2);
    // 认领前替读的首帧（若有）按正常上行处理，不丢首包。
    if let Some(Message::Binary(raw)) = prelude {
        on_uplink_binary(&raw, &mut up_enc, &mut up_pcm, &up_tx, &ble, ble_on);
    }
    loop {
        tokio::select! {
            inbound = stream.next() => {
                match inbound {
                    // 协议：/data 设备方向除首帧 Text identify 外恒为二进制；此后再收到
                    // Text（identify 重发等固件行为差异）一律忽略——绝不能断链，此前在
                    // 此把 Text 当致命帧 break，真机固件的 identify 直接把 data 链踢掉
                    // （设备侧表现为 close 1006 每 5s 重连）。
                    Some(Ok(Message::Binary(raw))) => {
                        on_uplink_binary(&raw, &mut up_enc, &mut up_pcm, &up_tx, &ble, ble_on);
                    }
                    Some(Ok(Message::Text(_))) => {}
                    _ => break,
                }
            }
            out = data_rx.recv() => {
                match out {
                    Some(frame) => {
                        if sink.send(Message::Binary(frame.into())).await.is_err() { break; }
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
    })
}

/// 处理一个上行二进制帧：PCM 累积编码（攒满 60ms 一包）/ 哨兵收口（残段补零 +
/// listen stop）/ BLE 广播与 GATT 采集（仅 ble_proxy 开启时翻译）。
fn on_uplink_binary(
    raw: &[u8],
    up_enc: &mut OpusFrameEncoder,
    up_pcm: &mut Vec<i16>,
    up_tx: &mpsc::UnboundedSender<IncomingFrame>,
    ble: &Arc<std::sync::Mutex<BleLink>>,
    ble_on: bool,
) {
    match frames::parse_uplink(raw) {
        UplinkFrame::MicPcm(pcm) => {
            push_pcm(up_pcm, pcm);
            while up_pcm.len() >= frames::MIC_FRAME_SAMPLES {
                let frame: Vec<f32> = up_pcm
                    .drain(..frames::MIC_FRAME_SAMPLES)
                    .map(|s| s as f32 / 32768.0)
                    .collect();
                match up_enc.encode_frame(&frame) {
                    Ok(opus) => {
                        let _ = up_tx.send(IncomingFrame::Binary(opus));
                    }
                    Err(e) => tracing::debug!("[echomuse] 上行编码失败（丢帧）: {e}"),
                }
            }
        }
        UplinkFrame::VadEnd | UplinkFrame::NoSpeech => {
            // 一轮收口：残段补零成整帧发最后一包，然后会话侧 listen stop。
            if !up_pcm.is_empty() {
                up_pcm.resize(frames::MIC_FRAME_SAMPLES, 0);
                let frame: Vec<f32> = up_pcm.drain(..).map(|s| s as f32 / 32768.0).collect();
                if let Ok(opus) = up_enc.encode_frame(&frame) {
                    let _ = up_tx.send(IncomingFrame::Binary(opus));
                }
            }
            let _ = up_tx.send(IncomingFrame::Text(
                r#"{"type":"listen","state":"stop"}"#.into(),
            ));
        }
        UplinkFrame::BleAdverts(payload) if ble_on => {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(payload) {
                if let Some(list) = v.get("adverts").and_then(|a| a.as_array()) {
                    ble.lock()
                        .expect("echomuse ble link 锁中毒")
                        .merge_adverts(list, super::registry::now_secs());
                }
            }
        }
        UplinkFrame::BleGatt(payload) if ble_on => {
            ble.lock()
                .expect("echomuse ble link 锁中毒")
                .on_gatt(payload);
        }
        // ble_proxy 关闭时按未知帧忽略（与能力未声明一致，行为零变化）。
        UplinkFrame::BleAdverts(_) | UplinkFrame::BleGatt(_) => {}
        UplinkFrame::Unknown(_) => {}
    }
}

/// 把 S16_LE 字节推入 i16 缓冲（截掉奇数尾巴）。
fn push_pcm(buf: &mut Vec<i16>, pcm: &[u8]) {
    let pairs = pcm.len() / 2;
    buf.reserve(pairs);
    for chunk in pcm[..pairs * 2].chunks_exact(2) {
        buf.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
}
