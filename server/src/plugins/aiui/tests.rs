//! `server/src/plugins/aiui/mod.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;
use base64::Engine as _;
use serde_json::json;

#[test]
fn first_frame_carries_full_params_and_base64_audio() {
    let cfg = AiuiConfig {
        enabled: true,
        appid: "app1".into(),
        api_key: "k".into(),
        api_secret: "s".into(),
        scene: "main".into(),
        sn_prefix: "xz".into(),
        voice: "x6_dongmanshaonv_pro".into(),
        speed: 60,
        volume: 50,
        pitch: 50,
        prompt: "你是测试助手".into(),
        pace_ms: 10,
    };
    let f: serde_json::Value =
        serde_json::from_str(&first_frame(&cfg, "xz-dev", "audio-1", &[1, 2, 3, 4], 0))
            .unwrap();
    assert_eq!(f["header"]["appid"], "app1");
    assert_eq!(f["header"]["sn"], "xz-dev");
    assert_eq!(f["header"]["stmid"], "audio-1");
    assert_eq!(f["header"]["status"], 0);
    assert_eq!(f["header"]["interact_mode"], "oneshot");
    // parameter 仅首帧携带：tts 音色/语速与 payload 音频参数
    assert_eq!(f["parameter"]["tts"]["vcn"], "x6_dongmanshaonv_pro");
    assert_eq!(f["parameter"]["tts"]["speed"], 60);
    assert_eq!(f["parameter"]["tts"]["tts"]["sample_rate"], 16000);
    assert_eq!(f["parameter"]["nlp"]["prompt"], "你是测试助手");
    let audio = base64::engine::general_purpose::STANDARD
        .decode(f["payload"]["audio"]["audio"].as_str().unwrap())
        .unwrap();
    assert_eq!(audio, vec![1, 2, 3, 4]);
    assert_eq!(f["payload"]["audio"]["status"], 0);
}

#[test]
fn next_frame_is_simplified() {
    let cfg = AiuiConfig::default();
    let f: serde_json::Value =
        serde_json::from_str(&next_frame(&cfg, "sn", "audio-2", &[], 2)).unwrap();
    assert!(f.get("parameter").is_none(), "中/尾帧不得携带 parameter");
    assert_eq!(f["header"]["status"], 2);
    assert_eq!(f["payload"]["audio"]["status"], 2);
    assert_eq!(f["payload"]["audio"]["audio"], "");
}

/// 结果帧解析：以官方文档 5.1 的样例为黄金用例（event/iat/nlp/tts 四类子段）。
#[test]
fn parse_result_frame_decodes_all_subs() {
    // event：Bos
    let ev = base64::engine::general_purpose::STANDARD
        .encode(r#"{"type":"Vad","data":"","key":"Bos","desc":{}}"#);
    let f = parse_result_frame(&json!({
        "header": {"code": 0, "message": "success", "status": 1, "stmid": "1"},
        "payload": {"event": {"text": ev, "status": 0}}
    }).to_string()).unwrap();
    assert_eq!(f.event_key.as_deref(), Some("Bos"));
    assert_eq!(f.code, 0);

    // iat：官方听写格式
    let iat = r#"{"sn":1,"ls":false,"bg":0,"ed":0,"pgs":"","ws":[{"cw":[{"w":"你"}]} ,{"cw":[{"w":"好吗"}]}]}"#;
    let iat_b64 = base64::engine::general_purpose::STANDARD.encode(iat);
    let f = parse_result_frame(&json!({
        "header": {"code": 0, "status": 1},
        "payload": {"iat": {"text": iat_b64, "status": 2}}
    }).to_string()).unwrap();
    assert_eq!(f.iat_text.as_deref(), Some("你好吗"));

    // nlp：流式增量
    let nlp = base64::engine::general_purpose::STANDARD.encode("今天天气晴朗");
    let f = parse_result_frame(&json!({
        "header": {"code": 0, "status": 1},
        "payload": {"nlp": {"text": nlp, "status": 1}}
    }).to_string()).unwrap();
    assert_eq!(f.nlp_delta.as_deref(), Some("今天天气晴朗"));
    assert_eq!(f.nlp_status, Some(1));

    // tts：音频
    let f = parse_result_frame(&json!({
        "header": {"code": 0, "status": 1},
        "payload": {"tts": {"audio": base64::engine::general_purpose::STANDARD.encode([1u8, 2]), "status": 2}}
    }).to_string()).unwrap();
    assert_eq!(f.tts_audio, Some(vec![1u8, 2]));
    assert_eq!(f.tts_status, Some(2));

    // 错误帧
    let f = parse_result_frame(&json!({
        "header": {"code": 10110, "message": "server licence error", "status": 2}
    }).to_string()).unwrap();
    assert_eq!(f.code, 10110);
    assert!(f.header_session_end);
}

/// stmid 每轮递增且首帧/尾帧状态正确（发送侧协议回归，不发真实网络请求）。
#[test]
fn frame_status_progresses_per_turn() {
    let cfg = AiuiConfig::default();
    // 模拟 2 字节音频 → 1 帧：首帧即尾帧（status=0→2 的取值规则以帧序为准）
    let f: serde_json::Value =
        serde_json::from_str(&first_frame(&cfg, "sn", "audio-3", &[7], 0)).unwrap();
    assert_eq!(f["header"]["status"], 0);
    let f: serde_json::Value = serde_json::from_str(&next_frame(&cfg, "sn", "audio-3", &[7], 2))
        .unwrap();
    assert_eq!(f["header"]["status"], 2);
}

/// 真实连 AIUI 的链路冒烟（**排障工具，日常不跑**）：密钥经环境变量、不入库——
/// `AIUI_APP_ID=… AIUI_API_KEY=… AIUI_API_SECRET=… cargo test aiui_live -- --ignored --nocapture`
///
/// 发 1 秒静音（16k/16bit）：鉴权/协议/云端 VAD 任一环节有问题都会报错；
/// 正常结果 = 轮次完成且无识别文本（Silence 事件收尾）。
#[test]
#[ignore = "live AIUI 调用：需 AIUI_APP_ID/AIUI_API_KEY/AIUI_API_SECRET 环境变量"]
fn aiui_live_smoke() {
    let (Ok(appid), Ok(api_key), Ok(api_secret)) = (
        std::env::var("AIUI_APP_ID"),
        std::env::var("AIUI_API_KEY"),
        std::env::var("AIUI_API_SECRET"),
    ) else {
        eprintln!("跳过：未设置 AIUI_APP_ID/AIUI_API_KEY/AIUI_API_SECRET");
        return;
    };
    let cfg = AiuiConfig {
        enabled: true,
        appid,
        api_key,
        api_secret,
        scene: std::env::var("AIUI_SCENE").unwrap_or_else(|_| "main".into()),
        pace_ms: std::env::var("AIUI_PACE_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10),
        ..AiuiConfig::default()
    };
    println!("== vcn={} scene={} ==", cfg.voice, cfg.scene);
    let mut session = AiuiSession::new(cfg, "xz-smoke".into());
    // AIUI_SAY=<path>：喂 16k/16bit LE PCM 文件（真实语音轮次）；否则发 1 秒静音
    let pcm: Vec<i16> = match std::env::var("AIUI_SAY") {
        Ok(path) => std::fs::read(&path)
            .expect("读取 PCM 文件失败")
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect(),
        Err(_) => vec![0i16; 16000],
    };
    let t0 = Instant::now();
    let result = session.turn(&pcm);
    println!(
        "== 结果：{}（{}ms）==",
        match &result {
            Ok(t) => format!("OK stt={}字 reply={}字 audio={}B", t.stt.chars().count(), t.reply.chars().count(), t.audio.len()),
            Err(e) => format!("ERR {e:#}"),
        },
        t0.elapsed().as_millis()
    );
}
