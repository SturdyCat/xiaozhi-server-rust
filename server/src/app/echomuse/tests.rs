//! EchoMuse 模块测试：帧编解码、ack/config 形状、`EchomuseTransport` 双向翻译。
//!
//! 传输翻译用 channel 直驱（不起真实 WebSocket）：模拟 inbound 任务的输出直接推
//! 上行 channel，断言会话侧看到正确的 xiaozhi 语义帧；下行方向反向断言 `0x02/0x03`。

use tokio::sync::mpsc;

use super::frames::*;
use super::transport::EchomuseTransport;
use super::{ack_message, config_push_with, ControlOut};
use crate::app::transport::{IncomingFrame, Transport};

// ---- 帧编解码 ----

#[test]
fn mic_pcm_roundtrip() {
    let pcm = [0u8, 1, 2, 3, 4, 5, 6, 7];
    let mut frame = vec![FRAME_MIC];
    frame.extend_from_slice(&pcm);
    assert_eq!(parse_uplink(&frame), UplinkFrame::MicPcm(&pcm));
}

#[test]
fn session_audio_strips_header() {
    // 0x07 头：session u32 BE + seq u16 BE，其后才是 PCM。
    let pcm = [9u8, 8, 7];
    let mut frame = vec![FRAME_SESSION_AUDIO, 0, 0, 0, 42, 0, 7];
    frame.extend_from_slice(&pcm);
    assert_eq!(parse_uplink(&frame), UplinkFrame::MicPcm(&pcm));
}

#[test]
fn session_audio_truncated_is_unknown() {
    // 不足 6 字节头的 0x07 按未知忽略（不把头字节当 PCM）。
    assert_eq!(
        parse_uplink(&[FRAME_SESSION_AUDIO, 0, 0]),
        UplinkFrame::Unknown(FRAME_SESSION_AUDIO)
    );
}

#[test]
fn vad_sentinels() {
    assert_eq!(parse_uplink(&[FRAME_VAD_END]), UplinkFrame::VadEnd);
    assert_eq!(parse_uplink(&[FRAME_NO_SPEECH]), UplinkFrame::NoSpeech);
}

#[test]
fn unknown_and_empty_frames() {
    assert_eq!(parse_uplink(&[0x2A]), UplinkFrame::Unknown(0x2A));
    // 下行方向专属的音乐帧 0x04/0x05 出现在上行也是未知（方向命名空间）：
    // 0x04/0x05 在上行是哨兵，这里验证的是真正的未知值。
    assert_eq!(parse_uplink(&[0x99]), UplinkFrame::Unknown(0x99));
    assert_eq!(parse_uplink(&[]), UplinkFrame::Unknown(0));
}

#[test]
fn speaker_frames_shape() {
    assert_eq!(frame_speaker(&[1, 2, 3]), vec![FRAME_SPEAKER, 1, 2, 3]);
    assert_eq!(frame_speaker_eos(), vec![FRAME_SPEAKER_EOS]);
}

// ---- ack / config 形状（能力协商是兼容性契约） ----

#[test]
fn ack_declares_supported_subset_only() {
    let ack = ack_message("dev-1", false);
    let v: serde_json::Value = serde_json::from_str(&ack).unwrap();
    assert_eq!(v["type"], "ack");
    assert_eq!(v["device_id"], "dev-1");
    let feats = v["features"].as_array().unwrap();
    // 不声明 listen_session / audio_mix / ble_connect —— 服务器不支持的照协议静默降级。
    let names: Vec<&str> = feats.iter().filter_map(|f| f.as_str()).collect();
    assert!(names.contains(&"mic") && names.contains(&"speaker"));
    assert!(!names.contains(&"listen_session"));
    assert!(!names.contains(&"audio_mix"));
    assert!(!names.contains(&"ble_connect"));
    assert!(!names.contains(&"ble_adverts_data"));
}

#[test]
fn config_push_enables_on_device_wake() {
    let msgs = config_push_with(&serde_json::json!({}));
    assert_eq!(msgs.len(), 1);
    let v: serde_json::Value = serde_json::from_str(&msgs[0]).unwrap();
    assert_eq!(v["type"], "config");
    assert_eq!(v["owwOnDevice"], true);
    // 设置合并覆盖基线；wakeWordEnabled 单独成条。
    let msgs = config_push_with(
        &serde_json::json!({"owwOnDevice": false, "wakeSound": true, "wakeWordEnabled": false}),
    );
    assert_eq!(msgs.len(), 2);
    let v: serde_json::Value = serde_json::from_str(&msgs[0]).unwrap();
    assert_eq!(v["owwOnDevice"], false);
    assert_eq!(v["wakeSound"], true);
    assert!(v.get("wakeWordEnabled").is_none());
    let v: serde_json::Value = serde_json::from_str(&msgs[1]).unwrap();
    assert_eq!(v["wakeWordEnabled"], false);
}

// ---- EchomuseTransport 翻译 ----

struct Harness {
    transport: EchomuseTransport,
    up_tx: mpsc::UnboundedSender<IncomingFrame>,
    ctl_rx: mpsc::UnboundedReceiver<ControlOut>,
    data_rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

fn harness() -> Harness {
    let (up_tx, up_rx) = mpsc::unbounded_channel();
    let (ctl_tx, ctl_rx) = mpsc::unbounded_channel();
    let (data_tx, data_rx) = mpsc::unbounded_channel();
    Harness {
        transport: EchomuseTransport::new(up_rx, ctl_tx, data_tx).unwrap(),
        up_tx,
        ctl_rx,
        data_rx,
    }
}

#[tokio::test]
async fn recv_translates_uplink_events() {
    let mut h = harness();
    h.up_tx
        .send(IncomingFrame::Text(
            r#"{"type":"listen","state":"start"}"#.into(),
        ))
        .unwrap();
    h.up_tx.send(IncomingFrame::Binary(vec![1, 2, 3])).unwrap();
    drop(h.up_tx); // 上行 channel 关闭 = 断开

    assert!(matches!(
        h.transport.recv().await,
        IncomingFrame::Text(s) if s.contains("\"state\":\"start\"")
    ));
    assert!(matches!(h.transport.recv().await, IncomingFrame::Binary(b) if b == vec![1, 2, 3]));
    // 粘滞 Closed：channel 已关，之后恒报 Closed。
    assert!(matches!(h.transport.recv().await, IncomingFrame::Closed));
    assert!(matches!(
        h.transport.try_recv(),
        Some(IncomingFrame::Closed)
    ));
}

#[tokio::test]
async fn send_binary_produces_speaker_pcm_chunks() {
    let mut h = harness();
    // 一包 Opus 在 default（无 sherpa）构建下解码为空——仍应走通不发帧；
    // 真实解码路径由 `--features sherpa` 集成覆盖，这里验证通道与帧封装不断。
    h.transport.send_binary(b"\x01\x02".to_vec()).await.unwrap();
    // default 构建：解码空 → 无 0x02；data 面应有 0 条消息。
    assert!(h.data_rx.try_recv().is_err());
}

#[tokio::test]
async fn tts_stop_emits_eos() {
    let mut h = harness();
    h.transport
        .send_text(r#"{"type":"tts_stop","session_id":""}"#.into())
        .await
        .unwrap();
    // tts stop → 0x03 EOS（default 构建下无积压 PCM，直接一条 EOS）。
    let eos = h.data_rx.try_recv().expect("应发出 EOS");
    assert_eq!(eos, frame_speaker_eos());
    // 其余 JSON 不下发（stt/llm 等设备不消费）。
    h.transport
        .send_text(r#"{"type":"stt","text":"你好"}"#.into())
        .await
        .unwrap();
    assert!(h.data_rx.try_recv().is_err());
}

#[tokio::test]
async fn close_requests_shutdown_and_sticks() {
    let mut h = harness();
    h.transport.close().await;
    match h.ctl_rx.try_recv() {
        Ok(ControlOut::Shutdown) => {}
        other => panic!("close 应发 Shutdown，实际 {other:?}"),
    }
    assert!(matches!(h.transport.recv().await, IncomingFrame::Closed));
}

#[tokio::test]
async fn try_recv_empty_is_none_not_closed() {
    let mut h = harness();
    assert!(h.transport.try_recv().is_none());
}

// ---- /em/status 状态查询 ----

use super::hub_tests::HUB_LOCK;

use super::{hub_insert, hub_remove, status_json, PendingDevice};

/// hub 是进程级单例，读写它的测试必须串行（否则 `status_disabled_shape` 会读到
/// 并行探针）。锁本体与登记助手见 [`super::hub_tests`]。

/// 借真实 hub 登记一台假设备，测试完清理（hub 是进程级单例，不能留残）。
pub(super) fn insert_probe(device_id: &str) {
    let (up_tx, _up_rx) = mpsc::unbounded_channel::<IncomingFrame>();
    let (_ctl_tx, _ctl_rx) = mpsc::unbounded_channel::<ControlOut>();
    let (_data_tx, data_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    hub_insert(
        device_id,
        PendingDevice {
            up_tx,
            data_rx: Some(data_rx),
            since: std::time::Instant::now(),
            ctl_tx: None,
            data_tx: None,
            ble: std::sync::Arc::new(std::sync::Mutex::new(super::ble::BleLink::default())),
        },
    );
}

#[test]
fn status_disabled_shape() {
    let _g = HUB_LOCK.lock().unwrap();
    // 关闭时也返回 200 语义的形状：enabled=false + 空 devices（该接口用于排查，不 404）。
    let v = status_json(false);
    assert_eq!(v["enabled"], false);
    assert_eq!(v["devices"].as_array().map(Vec::len), Some(0));
}

#[test]
fn status_enabled_reflects_hub_keys() {
    let _g = HUB_LOCK.lock().unwrap();
    // enabled 只透传配置；devices 取 hub 当前 key 集合（channel 直驱：不起真实 WS，
    // 直接登记/清理 hub，与 control/data 路由的 insert/remove 走同一函数）。
    insert_probe("dev-status-1");
    insert_probe("dev-status-2");
    let v = status_json(true);
    assert_eq!(v["enabled"], true);
    let ids: Vec<&str> = v["devices"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["device_id"].as_str())
        .collect();
    assert!(ids.contains(&"dev-status-1") && ids.contains(&"dev-status-2"));
    for d in v["devices"].as_array().unwrap() {
        assert!(d["connected_secs"].is_u64());
    }
    // 会话结束 hub_remove 后设备从状态里消失。
    hub_remove("dev-status-1");
    hub_remove("dev-status-2");
    let v = status_json(true);
    let ids: Vec<&str> = v["devices"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["device_id"].as_str())
        .collect();
    assert!(!ids.contains(&"dev-status-1") && !ids.contains(&"dev-status-2"));
}

// ---- 设备登记持久化（registry） ----

use super::registry::Registry;
use super::DeviceRegister;

/// 独立临时文件的 registry（不碰进程级 global；测试完删除落盘文件）。
pub(super) fn temp_registry(tag: &str) -> (Registry, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "echomuse-reg-test-{}-{}.json",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    (Registry::load(path.clone()), path)
}

pub(super) fn reg_load(device_id: &str, version: &str) -> DeviceRegister {
    serde_json::from_str(&format!(
        r#"{{"device_id":"{device_id}","version":"{version}","board":"echo-dot-2","base_os":"emOS 0.10","capabilities":["mic"]}}"#
    ))
    .unwrap()
}

#[test]
fn registry_upsert_and_roundtrip() {
    let (reg, path) = temp_registry("roundtrip");
    reg.upsert(&reg_load("dev-reg-1", "1.0"), 100);
    // 第二次接入：刷新版本/last_seen，保留 first_seen 与默认 label。
    reg.upsert(&reg_load("dev-reg-1", "1.1"), 200);
    reg.upsert(&reg_load("dev-reg-2", "2.0"), 300);

    let reload_path = path.clone();
    std::fs::rename(&path, &reload_path).unwrap();
    let reloaded = Registry::load(reload_path.clone());

    let d1 = reloaded.get("dev-reg-1").expect("落盘后应能读回");
    assert_eq!(d1.version, "1.1");
    assert_eq!(d1.first_seen, 100);
    assert_eq!(d1.last_seen, 200);
    assert_eq!(d1.label, "dev-reg-1");
    assert!(!d1.approved);
    assert_eq!(d1.board.as_deref(), Some("echo-dot-2"));
    assert_eq!(d1.base_os.as_deref(), Some("emOS 0.10"));
    // list 全表按 device_id 排序。
    let ids: Vec<String> = reloaded
        .list()
        .iter()
        .map(|d| d.device_id.clone())
        .collect();
    assert_eq!(ids, vec!["dev-reg-1".to_string(), "dev-reg-2".to_string()]);
    let _ = std::fs::remove_file(&reload_path);
}

#[test]
fn registry_remove_deletes_and_persists() {
    let (reg, path) = temp_registry("remove");
    reg.upsert(&reg_load("dev-mock", "1.0"), 100);
    reg.upsert(&reg_load("dev-real", "2.0"), 200);
    // 删除存在的记录：true，get/list 落盘后均不可见；别的记录不受影响。
    assert!(reg.remove("dev-mock"));
    let reloaded = Registry::load(path.clone());
    assert!(reloaded.get("dev-mock").is_none());
    assert!(reloaded.get("dev-real").is_some());
    assert_eq!(reloaded.list().len(), 1);
    // 再删同一条：false（HTTP 层转 404）。
    assert!(!reg.remove("dev-mock"));
    assert!(!reloaded.remove("dev-mock"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn registry_approve_sets_flag_and_label() {
    let (reg, path) = temp_registry("approve");
    reg.upsert(&reg_load("dev-ap-1", "1.0"), 100);
    let updated = reg
        .approve("dev-ap-1", Some(" 客厅音箱 "))
        .expect("存在应返回记录");
    assert!(updated.approved);
    assert_eq!(updated.label, "客厅音箱"); // trim 生效
                                           // 落盘往返：重建后 approved/label 仍在。
    let reloaded = Registry::load(path.clone());
    let d = reloaded.get("dev-ap-1").unwrap();
    assert!(d.approved);
    assert_eq!(d.label, "客厅音箱");
    // 空白 label 不改名；不存在返回 None（HTTP 层转 404）。
    assert_eq!(
        reloaded.approve("dev-ap-1", Some("  ")).unwrap().label,
        "客厅音箱"
    );
    assert!(reloaded.approve("dev-missing", None).is_none());
    let _ = std::fs::remove_file(&path);
}

// ---- mDNS 广播地址解析（纯动态探测，无配置项） ----

#[test]
fn advertise_ipv4_fallback_is_routable_or_loopback() {
    use super::advertise_ipv4;
    let ip = advertise_ipv4();
    assert!(
        ip.is_loopback() || ip.is_private(),
        "探测结果应为局域网地址或 loopback 回落，实际 {ip}"
    );
}
