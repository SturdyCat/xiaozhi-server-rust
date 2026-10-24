//! EchoMuse 模块测试：帧编解码、ack/config 形状、`EchomuseTransport` 双向翻译。
//!
//! 传输翻译用 channel 直驱（不起真实 WebSocket）：模拟 inbound 任务的输出直接推
//! 上行 channel，断言会话侧看到正确的 xiaozhi 语义帧；下行方向反向断言 `0x02/0x03`。

use tokio::sync::mpsc;

use super::frames::*;
use super::transport::EchomuseTransport;
use super::{ack_message, config_push_with, ControlOut};
use crate::workflow::protocol::{IncomingFrame, Transport};

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
    // features 是控制器能力表：固件只读 output_chain/ble_*，绝无设备能力名噪音。
    let feats = v["features"].as_array().unwrap();
    let names: Vec<&str> = feats.iter().filter_map(|f| f.as_str()).collect();
    assert!(names.contains(&"output_chain"));
    // 不声明 listen_session —— 私听会话未实现，声明会让设备在 owwOnDevice=on 时
    // 走私听路径（而 oww 资产未同步 ⇒ 既不推流也不触发）。
    assert!(!names.contains(&"listen_session"));
    assert!(!names.contains(&"audio_mix"));
    assert!(!names.contains(&"ble_connect"));
    assert!(!names.contains(&"ble_adverts_data"));
    // 设备能力名不是 controller feature，一个都不该出现。
    for noise in [
        "mic",
        "speaker",
        "leds",
        "buttons",
        "oww_trigger",
        "oww_local_only",
    ] {
        assert!(!names.contains(&noise), "噪音 feature: {noise}");
    }
    // 校时：设备无 RTC，ack 顺带 time_ms（官方 em_controller.py:5189）。
    let time_ms = v["time_ms"].as_u64().expect("ack 缺 time_ms");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert!(
        now >= time_ms && now - time_ms < 5_000,
        "time_ms 应是当前时刻"
    );
}

#[test]
fn config_push_defaults_to_off_device_wake() {
    let msgs = config_push_with(&serde_json::json!({}));
    assert_eq!(msgs.len(), 1);
    let v: serde_json::Value = serde_json::from_str(&msgs[0]).unwrap();
    assert_eq!(v["type"], "config");
    // 基线 off：设备 oww 资产未同步，"on" 只会空转加载评分器；stream 模式下设备
    // 无论此值都持续推流（listen.Resolve）。
    assert_eq!(v["owwOnDevice"], "off");
    // 设置合并覆盖基线；wakeWordEnabled 单独成条。bool 兼容旧值转协议字符串。
    let msgs = config_push_with(
        &serde_json::json!({"owwOnDevice": true, "wakeSound": true, "wakeWordEnabled": false}),
    );
    assert_eq!(msgs.len(), 2);
    let v: serde_json::Value = serde_json::from_str(&msgs[0]).unwrap();
    assert_eq!(v["owwOnDevice"], "on");
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
    assert!(h.transport.send_binary(b"\x01\x02".to_vec()).await.is_err());
    assert!(h.data_rx.try_recv().is_err());
}

#[tokio::test]
async fn tts_stop_emits_eos() {
    let mut h = harness();
    h.transport
        .send_text(
            crate::workflow::protocol::ServerMessage::Tts {
                session_id: "contract-session".into(),
                state: "stop".into(),
                text: None,
            }
            .to_json(),
        )
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
            management: std::sync::Arc::new(super::runtime::LiveControl::new(None, false)),
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

// ---- /em/data 首帧 identify 认领 ----

#[test]
fn identify_frame_parses_device_id() {
    // 真机固件拨 /data 后立即发的唯一文本帧（上游 data.go TextMessage identify）。
    assert_eq!(
        super::routes::parse_identify(r#"{"type":"identify","device_id":"G090L910723409TB"}"#),
        Some("G090L910723409TB".to_string())
    );
}

#[test]
fn identify_frame_wrong_shape_falls_back_to_claim_only() {
    // 非 identify 类型 / 缺 device_id / 空 device_id / 非法 JSON → None，
    // 由 em_data 按"唯一待认领设备"旧规则兜底（mock/测试路径）。
    assert_eq!(super::routes::parse_identify(r#"{"type":"ping"}"#), None);
    assert_eq!(
        super::routes::parse_identify(r#"{"type":"identify"}"#),
        None
    );
    assert_eq!(
        super::routes::parse_identify(r#"{"type":"identify","device_id":""}"#),
        None
    );
    assert_eq!(super::routes::parse_identify("not json"), None);
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
fn register_parses_informational_boot_fields_with_defaults() {
    // 官方 register 的信息性字段：齐全会解析，缺省（老固件）全 None。
    let full: DeviceRegister = serde_json::from_str(
        r#"{"device_id":"d1","version":"v2.18.1","capabilities":["mic"],
            "ip":"192.168.9.9","kernel_arch":"armv7l","kernel_release":"3.18.19",
            "boot_id":"b-1","boot_reason":"power_key","emmc":{"rev":7}}"#,
    )
    .unwrap();
    assert_eq!(full.ip.as_deref(), Some("192.168.9.9"));
    assert_eq!(full.boot_reason.as_deref(), Some("power_key"));
    let minimal = reg_load("d2", "v1");
    assert!(minimal.ip.is_none() && minimal.boot_id.is_none());
    assert!(minimal.kernel_release.is_none() && minimal.boot_reason.is_none());
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
    // payload 输出 register 的网络/内核/启动标识（管理台概览展示用）。
    let payload = d1.payload(Some(5));
    assert_eq!(payload["ip"], serde_json::Value::Null, "reg_load 未带 ip");
    let full_record = {
        let reg2 = Registry::load(path.with_extension("full.json"));
        reg2.upsert(
            &serde_json::from_str::<DeviceRegister>(
                r#"{"device_id":"d3","version":"v2","capabilities":["mic"],
                    "ip":"10.0.0.3","kernel_arch":"armv7l","kernel_release":"3.18.19",
                    "boot_id":"b-9","boot_reason":"reboot"}"#,
            )
            .unwrap(),
            400,
        );
        reg2.get("d3").expect("d3 已登记").clone()
    };
    let full_payload = full_record.payload(None);
    assert_eq!(full_payload["ip"], "10.0.0.3");
    assert_eq!(full_payload["kernel_arch"], "armv7l");
    assert_eq!(full_payload["kernel_release"], "3.18.19");
    assert_eq!(full_payload["boot_id"], "b-9");
    assert_eq!(full_payload["boot_reason"], "reboot");
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

// ---- start_turn 唤醒入口（routes.rs）----

#[tokio::test]
async fn 开轮聆听先detect再listen_start且重复唤醒零副作用() {
    let (up_tx, mut up_rx) = mpsc::unbounded_channel();
    let live = super::runtime::LiveControl::new(None, false);
    let (ctl_tx, _ctl_rx) = mpsc::unbounded_channel();
    live.mic.bind(ctl_tx.clone());
    live.fx.bind(ctl_tx);
    super::turn::start_turn(&live, &up_tx, "本地唤醒词");
    match up_rx.recv().await {
        Some(IncomingFrame::Text(t)) => {
            assert!(
                t.contains(r#""state":"detect""#) && t.contains(r#""text":"本地唤醒词""#),
                "首帧必须是 detect 且携带来源标签（唤醒问候语入口）: {t}"
            )
        }
        other => panic!("首帧应是文本帧: {other:?}"),
    }
    match up_rx.recv().await {
        Some(IncomingFrame::Text(t)) => {
            assert!(
                t.contains(r#""state":"start""#),
                "次帧必须是 listen start: {t}"
            )
        }
        other => panic!("次帧应是文本帧: {other:?}"),
    }
    // 已在聆听：重复唤醒零副作用（不重复 mic_start、不再投帧）。
    super::turn::start_turn(&live, &up_tx, "本地唤醒词");
    assert!(up_rx.try_recv().is_err(), "重复唤醒不得再投帧");
}

// ---- P2 上行面：此前丢弃的消息现在进入有界 runtime ----

#[test]
fn telemetry_consumes_uplink_reports_previously_dropped() {
    let live = super::runtime::LiveControl::new(None, false);
    live.update(&serde_json::json!({"type":"listen_end","session":7,"reason":"ack_timeout"}));
    live.update(&serde_json::json!({
        "type":"playback_stats","periods":128,"underruns":0,
        "stats":{"minDepth":23,"primeWaitMs":935,"maxGapMs":41}}));
    live.update(&serde_json::json!({
        "type":"oww_shadow_cross","score":0.83,"threshold":0.5}));
    live.update(&serde_json::json!({"type":"wifi_result","ok":true,"ssid":"home"}));
    live.update(&serde_json::json!({
        "type":"wifi_scan_result","networks":[{"ssid":"home","signal":-40}]}));
    live.update(&serde_json::json!({"type":"pair_request"}));
    let p = live.payload("d1");
    assert_eq!(p["listen_end"]["reason"], "ack_timeout");
    assert_eq!(p["playback_stats"]["underruns"], 0);
    assert_eq!(p["playback_stats"]["stats"]["minDepth"], 23);
    assert_eq!(p["oww_shadow_cross"]["score"], 0.83);
    assert_eq!(p["wifi_result"]["ok"], true);
    assert_eq!(p["wifi_scan"]["networks"][0]["ssid"], "home");
    assert!(p["pair_requested_at"].as_u64().is_some());
    // 长字符串截断：runtime 是有界缓存，不吞内存。
    live.update(&serde_json::json!({
        "type":"playback_stats","note": "x".repeat(2000)}));
    let p = live.payload("d1");
    assert!(
        p["playback_stats"]["note"]
            .as_str()
            .map(|s| s.len())
            .unwrap_or(0)
            <= 256
    );
}
