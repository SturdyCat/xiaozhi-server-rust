// hello 演示插件（官方默认实现）：走完 manifest/register/handle-event/handle-action/self-test
// 全链路。P1 起经 guest SDK 使用契约绑定与构建器（契约与宿主同源）。
// 构建目标 wasm32-unknown-unknown（纯逻辑、无 WASI），wasm-tools component new 组件化。

#[global_allocator]
static ALLOC: guest_alloc::GrowingBump = guest_alloc::GrowingBump::new();

use xiaozhi_plugin_sdk::builder::{FieldSpec, ManifestBuilder, RegistrationBuilder};
use xiaozhi_plugin_sdk::exports::lifecycle::Guest;
use xiaozhi_plugin_sdk::fallible;
use xiaozhi_plugin_sdk::xiaozhi::plugin::types;

struct Component;

/// 夹具级 JSON 字段子串提取（"key":"value" / "key":123）——完整解析交给真插件用 serde_json。
fn json_str_field(payload: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = payload.find(&marker)? + marker.len();
    let end = payload[start..].find('"')? + start;
    Some(payload[start..end].to_string())
}

fn json_int_field(payload: &str, key: &str) -> Option<i64> {
    let marker = format!("\"{key}\":");
    let start = payload.find(&marker)? + marker.len();
    let tail = &payload[start..];
    let end = tail
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(tail.len());
    tail[..end].parse().ok()
}

impl Guest for Component {
    fn manifest() -> types::PluginManifest {
        ManifestBuilder::new("hello.official", "Hello 演示插件")
            .expect("静态构建：id/title 已内联校验")
            .version("0.1.2")
            .description("P1b 冒烟：SDK 构建器 + kv 读写闭环 + http-get 两段式回灌")
            .permission("kv")
            .expect("kv 权限")
            .permission("lan-socket")
            .expect("lan-socket 权限")
            .permission("http-out:example.com")
            .expect("http-out 权限")
            .implements("demo.greeter")
            .expect("扩展点")
            .implements("asr.provider")
            .expect("扩展点（P4a 探针：utterance 级远端引擎演示）")
            .implements("tts.provider")
            .expect("扩展点（P4 策略控制演示）")
            .implements("llm.provider")
            .expect("扩展点（P4 策略控制演示）")
            .implements("vad.provider")
            .expect("扩展点（P4d 策略控制演示）")
            .implements("connector.tuya")
            .expect("扩展点（P5 控制策略演示）")
            .implements("device.echomuse")
            .expect("扩展点（P6 会话入口策略演示）")
            .implements("prompt.soul")
            .expect("扩展点（P7 有界人格策略演示）")
            .implements("wakeup.strategy")
            .expect("扩展点（P7 唤醒策略演示）")
            .implements("debug.policy")
            .expect("扩展点（P7 debug 策略演示）")
            .implements("skill.policy")
            .expect("扩展点（P7 skills 策略演示）")
            .implements("memory.policy")
            .expect("扩展点（P7 memory 策略演示）")
            .implements("nlu.catalog")
            .expect("扩展点（P7 NLU 目录策略演示）")
            .official(true)
            .build()
            .expect("静态构建")
    }

    fn register() -> types::Registration {
        RegistrationBuilder::new()
            .menu_entry("nav.main", "hello", "Hello", "插件", 100, "settings")
            .expect("静态构建")
            .config_field(
                FieldSpec::str("greeting", "问候语")
                    .hint("你好")
                    .help("动作 greet 的默认称呼")
                    .build(),
            )
            .expect("静态构建")
            .action("greet", "打招呼", false, "返回一句问候")
            .expect("静态构建")
            .action(
                "recall",
                "回忆",
                false,
                "读回 kv 里最近一次 demo.kv 事件载荷",
            )
            .expect("静态构建")
            .build()
    }

    fn handle_event(event: String, payload: String) -> Result<Vec<types::Effect>, String> {
        xiaozhi_plugin_sdk::xiaozhi::plugin::host::log("debug", &format!("event={event}"));
        match event.as_str() {
            "demo.echo" => Ok(vec![types::Effect::Log(types::EffectLog {
                level: "info".to_string(),
                message: format!("echo:{payload}"),
            })]),
            "demo.kv" => Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                key: "last".to_string(),
                value: payload.into_bytes(),
            })]),
            // http.probe：payload = 完整 URL（权限门要求 host=example.com）
            "http.probe" => Ok(vec![types::Effect::HttpGet(types::EffectHttpGet {
                request_id: "probe-1".to_string(),
                url: payload,
            })]),
            // http.loop：回灌后再发一次请求（宿主深度上限的演练载体）
            "http.loop" => Ok(vec![types::Effect::HttpGet(types::EffectHttpGet {
                request_id: "loop".to_string(),
                url: payload,
            })]),
            // http.responses：宿主按步批量回灌（数组）。存 kv 供测试断言；loop 再发一次
            "http.responses" => {
                let mut effects = vec![types::Effect::KvPut(types::EffectKvPut {
                    key: "last-http".to_string(),
                    value: payload.clone().into_bytes(),
                })];
                // 无 JSON 库的夹具级判别：payload 形如 {"id":"loop",...}
                if payload.contains("\"id\":\"loop\"") {
                    effects.push(types::Effect::HttpGet(types::EffectHttpGet {
                        request_id: "loop".to_string(),
                        url: "https://example.com/again".to_string(),
                    }));
                }
                Ok(effects)
            }
            // P4a：asr.provider 探针（utterance 级）。payload JSON =
            // {"req_id":"...","sample_rate":16000,"samples":<n>}（无 serde_json 的夹具：
            // 子串提取同 loop 判别惯例）。结果经 kv 通道返回（weather 同款结果协议）。
            "asr.transcribe" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                let samples = json_int_field(&payload, "samples").unwrap_or(0);
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("asr-result:{req_id}"),
                    // 结果协议与 weather 一致：JSON {"text":"..."}
                    value: format!("{{\"text\":\"你好世界（来自 wasm ASR，{samples} 采样）\"}}")
                        .into_bytes(),
                })])
            }
            // P4：TTS/LLM 只返回控制决策；原生宿主继续负责 PCM、HTTP 与 SSE。
            "tts.prepare" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("tts-result:{req_id}"),
                    value: b"{\"route\":\"native\"}".to_vec(),
                })])
            }
            "llm.prepare" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("llm-result:{req_id}"),
                    value: b"{\"route\":\"native\"}".to_vec(),
                })])
            }
            "vad.prepare" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("vad-result:{req_id}"),
                    value: b"{\"route\":\"native\",\"threshold\":0.65,\"min_silence_duration\":0.4,\"min_speech_duration\":0.3}".to_vec(),
                })])
            }
            // P5：只选择候选索引，native 宿主继续负责 local_key、DP 与 LAN socket。
            "connector.tuya.prepare" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                let index = json_int_field(&payload, "index").unwrap_or(0);
                let action = json_str_field(&payload, "action").unwrap_or_else(|| "on".to_string());
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("tuya-result:{req_id}"),
                    value: format!("{{\"route\":\"native\",\"candidate_index\":{index},\"action\":\"{action}\"}}").into_bytes(),
                })])
            }
            // P5b：仅产生高层 LAN 命令；句柄/DP/value 均受宿主边界校验。
            "connector.tuya.status" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                Ok(vec![types::Effect::LanStatus(types::EffectLanStatus {
                    request_id: req_id,
                    device_handle: "fixture-handle".to_string(),
                })])
            }
            "connector.tuya.set-dp" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                Ok(vec![types::Effect::LanSetDp(types::EffectLanSetDp {
                    request_id: req_id,
                    device_handle: "fixture-handle".to_string(),
                    dp: "1".to_string(),
                    value_json: "true".to_string(),
                })])
            }
            "device.echomuse.prepare" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                let welcome = json_str_field(&payload, "welcome").unwrap_or_default();
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("echomuse-result:{req_id}"),
                    value: format!("{{\"route\":\"native\",\"welcome\":\"{welcome}\"}}")
                        .into_bytes(),
                })])
            }
            // P7a：只返回 native 问候目录中的索引，不生成文本或音频。
            "wakeup.prepare" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("wakeup-result:{req_id}"),
                    value: b"{\"route\":\"native\",\"greeting_index\":0}".to_vec(),
                })])
            }
            // P7：只返回受限人格控制项；完整 instructions 与变量仍由 native 组装。
            "prompt.soul.prepare" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("soul-result:{req_id}"),
                    value: "{\"route\":\"native\",\"tone\":\"温和、简洁\",\"max_sentences\":2,\"max_chars\":40}".as_bytes().to_vec(),
                })])
            }
            // P7d：debug policy only returns bounded native presentation controls.
            e if e.starts_with("debug.") && e.ends_with(".prepare") => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("debug-result:{req_id}"),
                    value: br#"{"route":"native","max_items":32,"redact":true}"#.to_vec(),
                })])
            }
            // P7e：skill policy only returns a bounded native admission decision.
            "skill.policy.prepare" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("skill-result:{req_id}"),
                    value: br#"{"route":"native","allow":true,"max_steps":2}"#.to_vec(),
                })])
            }
            // P7f：memory policy only returns bounded native recall budgets.
            "memory.policy.prepare" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("memory-result:{req_id}"),
                    value: br#"{"route":"native","allow":true,"max_items":4,"max_chars":512}"#.to_vec(),
                })])
            }
            // P7g：nlu catalog may filter catalog presentation only; recognition stays native.
            "nlu.catalog.prepare" => {
                let req_id = json_str_field(&payload, "req_id").unwrap_or_default();
                Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                    key: format!("nlu-catalog-result:{req_id}"),
                    value: br#"{"route":"native","indices":[1,0]}"#.to_vec(),
                })])
            }
            // 会话观察者挂钩（P2d）：hook.<position> 事件 payload 原样落 kv 供断言
            e if e.starts_with("hook.") => Ok(vec![types::Effect::KvPut(types::EffectKvPut {
                key: "hook-last".to_string(),
                value: payload.into_bytes(),
            })]),
            other => Err(format!("未知事件：{other}")),
        }
    }

    fn handle_action(action_id: String, payload: String) -> Result<String, String> {
        match action_id.as_str() {
            // 运行期载荷走可失败分配：OOM = 业务性 Err（不是陷阱降级）
            "greet" => {
                let mut out =
                    fallible::try_string("你好，", "greet 前缀").map_err(|e| e.to_string())?;
                fallible::try_push_str(&mut out, &payload, "greet 追加")
                    .map_err(|e| e.to_string())?;
                fallible::try_push_str(&mut out, "！", "greet 结尾").map_err(|e| e.to_string())?;
                Ok(out)
            }
            // 宿主 kv-get 导入：读回 demo.kv 事件写入的值（有 kv 权限 → 放行）
            "recall" => match xiaozhi_plugin_sdk::xiaozhi::plugin::host::kv_get("last") {
                Some(bytes) => String::from_utf8(bytes).map_err(|e| format!("非 UTF-8：{e}")),
                None => Ok("无缓存（尚未发生 demo.kv 事件）".to_string()),
            },
            other => Err(format!("未知动作：{other}")),
        }
    }

    fn self_test() -> Result<Vec<types::TestResult>, String> {
        Ok(vec![types::TestResult {
            name: "roundtrip".to_string(),
            ok: true,
            detail: "ok".to_string(),
        }])
    }
}

xiaozhi_plugin_sdk::export!(Component);
