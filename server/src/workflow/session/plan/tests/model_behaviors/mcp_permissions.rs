use super::*;
use crate::workflow::mcp::{discovery::McpToolInfo, registry::ProbeResult};

#[tokio::test]
async fn general_plan_and_legacy_mcp_actions_require_positive_device_advertisement() {
    for plan in [false, true] {
        // Missing snapshot / failed discovery / known absent / known present.
        for capability in 0..4 {
            let mut wire = Wire {
                mcp_response: Some(json!({"result":{"content":[],"isError":false}})),
                ..Default::default()
            };
            let mut cfg = config(vec![binding("nlu.model", "nlu", 0, "stop")]);
            let action = if plan {
                json!({"kind":"plan","reply":"done","plan":{"steps":[
                    {"api":"devices.call","args":{"tool":"self.test.action","arguments":{}}}
                ]}})
            } else {
                json!({"kind":"mcp:self.test.action","reply":"done"})
            };
            cfg.nlu = serde_json::from_value(json!({
                "enabled":true,"bert_enabled":true,"slots":[],
                "intents":[{"name":"device_action","templates":["执行设备"],"action":action}]
            }))
            .unwrap();
            let mut s = session(&mut wire, cfg, Arc::new(Memory::default()));
            s.mcp_enabled = true;
            let classifications = install_model(&mut s, "device_action", 0.99);
            if capability != 0 {
                s.engines.mcp.record(ProbeResult {
                    device_id: s.device_id.clone(),
                    session_id: s.session_id.clone(),
                    probed_at_ms: 0,
                    board: String::new(),
                    firmware_version: String::new(),
                    protocol_version: String::new(),
                    error: (capability == 1).then(|| "discovery failed".into()),
                    tools: if capability == 3 {
                        vec![McpToolInfo {
                            name: "self.test.action".into(),
                            description: String::new(),
                            input_schema: json!({"type":"object","properties":{}}),
                            user_only: false,
                        }]
                    } else {
                        vec![]
                    },
                });
            }
            prime_audio(&s, "done");
            assert!(s.handle_skill_gate("执行设备").await);
            assert_eq!(classifications.load(Ordering::Relaxed), 1);
            let messages: Vec<Value> = s
                .transport
                .sent
                .iter()
                .map(|raw| serde_json::from_str(raw).unwrap())
                .collect();
            let calls: Vec<_> = messages
                .iter()
                .filter(|v| v["payload"]["method"] == "tools/call")
                .collect();
            assert_eq!(calls.len(), usize::from(capability == 3));
            assert_eq!(s.transport.audio_frames, usize::from(capability == 3));
            assert!(
                !s.close_requested,
                "general action must not use end-session fallback"
            );
            if capability != 3 {
                assert!(
                    s.transport.sent.is_empty(),
                    "no advertised permission: no call and no success receipt"
                );
            } else {
                assert_eq!(calls[0]["payload"]["params"]["name"], "self.test.action");
                let request = messages
                    .iter()
                    .position(|v| v["payload"]["method"] == "tools/call")
                    .unwrap();
                let receipt = messages
                    .iter()
                    .position(|v| v["type"] == "tts" && v["state"] == "start")
                    .unwrap();
                assert!(request < receipt);
            }
        }
    }
}
