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
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
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
    hub_remove, session_params, ControlOut, DataOut, DeviceRegister,
};

/// data 面入站消息认领失败时的日志前缀（诊断"只连了 data 没 control"类配置错误）。
const CLAIM_FAIL: &str = "[echomuse] /em/data 无待认领设备（control 未就绪、已认领或多台设备并存）";

/// 挂载 EchoMuse 路由（`[echomuse].enabled=true` 时由 [`crate::app::ws::router`] 调）。
pub fn em_routes() -> axum::Router<Arc<Engines>> {
    use axum::routing::get;
    axum::Router::new()
        .route("/em/control", get(em_control))
        .route("/em/data", get(em_data))
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
) -> Response {
    let cfg = &engines.config.echomuse;
    if !cfg.enabled {
        return (StatusCode::NOT_FOUND, "echomuse disabled").into_response();
    }
    if !token_ok(&headers, &cfg.token) {
        return (StatusCode::UNAUTHORIZED, "bad token").into_response();
    }
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = control_session(socket, engines).await {
            tracing::warn!("[echomuse] control 会话异常: {e}");
        }
    })
}

/// control 主体：register → ack → 起 control 任务 → 进会话主循环。
async fn control_session(socket: WebSocket, engines: Arc<Engines>) -> anyhow::Result<()> {
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
    hub_insert(
        &device_id,
        super::PendingDevice {
            up_tx: up_tx.clone(),
            data_rx: Some(data_rx),
            since: std::time::Instant::now(),
            // 管理面下发 reboot/wifi_change 的通道；control 会话自己持原件。
            ctl_tx: Some(ctl_tx.clone()),
            // 蓝牙代理 GATT 请求（0x08）的管理面下发口；control 会话自己持原件。
            data_tx: Some(data_tx.clone()),
            ble: ble.clone(),
        },
    );

    // control 任务：收设备事件（唤醒/按钮/旧固件 ble_adverts）+ 发 mic_start 等出站。
    let ctl_task = tokio::spawn(control_loop(sink, stream, ctl_rx, up_tx, ble, ble_on));

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
    hub_remove(&device_id);
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
) {
    // 当前是否处于一轮聆听（唤醒后到哨兵/停止之间）。
    let mut listening = false;
    loop {
        tokio::select! {
            inbound = stream.next() => {
                let Some(Ok(msg)) = inbound else { break };
                if let Message::Text(t) = msg {
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
/// （协议规定未知类型双向忽略；listen_state/stats/log/pong 等当前不消费）。
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
        // 认领：显式 ?device_id=（测试/同 IP 多设备）优先；否则仅当恰好一台未认领时
        // 取它（单设备家庭场景的默认路径）。协议允许按对端地址匹配，真机多设备联调
        // 时再按地址消歧。
        let claimed = match query.get("device_id") {
            Some(id) => hub_claim_data(id),
            None => hub_claim_only(),
        };
        let Some(claim) = claimed else {
            tracing::warn!("{CLAIM_FAIL}");
            return;
        };
        let ble_on = engines.config.echomuse.ble_proxy;
        data_loop(socket, claim, ble_on).await;
    })
}

/// data 任务：上行帧翻译（PCM→Opus、哨兵→listen stop、蓝牙代理 0x06/0x08 采集）
/// + 下行帧直发。
async fn data_loop(socket: WebSocket, claim: super::DataClaim, ble_on: bool) {
    let super::DataClaim {
        up_tx,
        mut data_rx,
        ble,
    } = claim;
    let (mut sink, mut stream) = socket.split();
    let mut up_enc = match OpusFrameEncoder::new(frames::MIC_SAMPLE_RATE) {
        Ok(enc) => enc,
        Err(e) => {
            tracing::warn!("[echomuse] 上行编码器创建失败: {e}");
            return;
        }
    };
    // 上行 PCM 暂存：攒满一个 60ms 帧（960 样本）编一包 Opus。
    let mut up_pcm: Vec<i16> = Vec::with_capacity(frames::MIC_FRAME_SAMPLES * 2);
    loop {
        tokio::select! {
            inbound = stream.next() => {
                let Some(Ok(Message::Binary(raw))) = inbound else { break };
                match frames::parse_uplink(&raw) {
                    UplinkFrame::MicPcm(pcm) => {
                        push_pcm(&mut up_pcm, pcm);
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
                            let frame: Vec<f32> =
                                up_pcm.drain(..).map(|s| s as f32 / 32768.0).collect();
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
                        ble.lock().expect("echomuse ble link 锁中毒").on_gatt(payload);
                    }
                    // ble_proxy 关闭时按未知帧忽略（与能力未声明一致，行为零变化）。
                    UplinkFrame::BleAdverts(_) | UplinkFrame::BleGatt(_) => {}
                    UplinkFrame::Unknown(_) => {}
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
    // 链路收尾：丢弃全部待应答 GATT 请求（等待方立即得到"离线"）并清空采集数据。
    ble.lock().expect("echomuse ble link 锁中毒").reset();
}

/// 把 S16_LE 字节推入 i16 缓冲（截掉奇数尾巴）。
fn push_pcm(buf: &mut Vec<i16>, pcm: &[u8]) {
    let pairs = pcm.len() / 2;
    buf.reserve(pairs);
    for chunk in pcm[..pairs * 2].chunks_exact(2) {
        buf.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
}
