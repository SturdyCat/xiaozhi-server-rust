use super::*;

/// NLU iot 域（`tuya.control`）会话分支回归：业务失败走 speech 话术收尾，
/// 会话正常延续——测试引擎的 tuya 目录为空，任何设备名都未找到。
#[tokio::test]
async fn tuya_control_not_found_speaks_and_session_continues() {
    let mut cfg = config(vec![binding("nlu.model", "nlu", 0, "stop")]);
    cfg.nlu.intents = serde_json::from_value(json!([{
        "name":"iot_device_on","domain":"iot","templates":["打开{device_name}"],
        "action":{"kind":"plan","reply":"","plan":{"conditions":[],"steps":[
            {"api":"tuya.control","args":{"name":"{{slots.device_name}}","action":"on"}},
            {"api":"replies.say","args":{"text":"{{steps.0.speech}}"}}
        ]}}}
    ]))
    .unwrap();
    cfg.nlu.slots =
        serde_json::from_value(json!([{"name":"device_name","values":[],"optional":false}]))
            .unwrap();
    let mut wire = Wire::default();
    let mut s = session(&mut wire, cfg, Arc::new(Memory::default()));
    let count = install_model(&mut s, "iot_device_on", 0.99);
    let expected = "没有找到叫「不存在插座」的设备，可以在管理台确认设备名称或别名后再试。";
    prime_audio(&s, expected);
    assert!(s.handle_skill_gate("打开不存在插座").await);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert_eq!(
        s.transport.audio_frames, 1,
        "voice_control 的未找到话术必须原样播报"
    );
    // 失败话术不是断开对话：会话照常延续。
    assert!(!s.close_requested);
    assert!(s.skills.tools().is_empty());
}
