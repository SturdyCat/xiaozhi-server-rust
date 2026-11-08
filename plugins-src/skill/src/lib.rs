// skill.official：技能准入策略。
//
// 新架构下 server 只做系统底座，重构前的「技能准入策略」功能以插件形式提供。
// 执行模型遵循平台铁律：宿主派发事件 → 插件同步返回 Vec<Effect>（纯决策 + 效果）→
// 宿主过权限门后异步执行，并从插件隔离 kv 的 `<结果键>:<req_id>` 读回有界 JSON。
// 数据面（PCM、模型推理、设备协议、HTTP/SSE）一律留在原生宿主，插件不持有 I/O 句柄。

#[global_allocator]
static ALLOC: guest_alloc::GrowingBump = guest_alloc::GrowingBump::new();

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::builder::{FieldSpec, ManifestBuilder, RegistrationBuilder};
use xiaozhi_plugin_sdk::exports::lifecycle::Guest;
use xiaozhi_plugin_sdk::xiaozhi::plugin::{host_core as host, types};

/// 插件 id：必须与上传请求 id、manifest.toml 登记项完全一致。
const PLUGIN_ID: &str = "skill.official";
/// 结果通道前缀：宿主按 `<prefix>:<req_id>` 读回本插件的决策。
const RESULT_PREFIX: &str = "skill-result";

struct Component;

/// 从开放形状载荷中提取 req_id（宿主为每次调用生成，结果必须写回同一 id）。
fn req_id_of(payload: &str) -> String {
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|v| v.get("req_id").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// 解析载荷；非法 JSON 属于业务性错误（不 panic、不降级）。
fn payload_of(payload: &str) -> Result<Value, String> {
    serde_json::from_str::<Value>(payload).map_err(|e| format!("载荷非法：{e}"))
}

/// 统一结果效果：把有界决策 JSON 写进 `<prefix>:<req_id>`。
fn result(value: &Value, req_id: &str) -> types::Effect {
    types::Effect::KvPut(types::EffectKvPut {
        key: format!("{}:{}", RESULT_PREFIX, req_id),
        value: value.to_string().into_bytes(),
    })
}

/// 要求 req_id 存在；缺失时终止本次决策，宿主显式失败（fail-closed）。
fn checked_req_id(payload: &str) -> Result<String, String> {
    let rid = req_id_of(payload);
    if rid.is_empty() {
        return Err("缺少 req_id".to_string());
    }
    Ok(rid)
}

impl Guest for Component {
    fn manifest() -> types::PluginManifest {
        ManifestBuilder::new(PLUGIN_ID, "技能准入策略")
            .expect("静态构建：id/title 已内联校验")
            .version("0.1.0")
            .description("skill.policy 扩展点：技能步骤准入与步数预算")
            .permission("kv")
            .expect("kv 权限")
            .implements("skill.policy")
            .expect("扩展点")
            .official(true)
            .build()
            .expect("静态构建")
    }

    fn register() -> types::Registration {
        RegistrationBuilder::new()
            .menu_entry("nav.main", "skill-policy", "技能策略", "插件", 119, "settings")
            .expect("静态构建")
            .config_field(
                FieldSpec::int("max_steps", "最大步数")
                    .hint("2")
                    .help("1..=32")
                    .build(),
            )
            .expect("静态构建")
            .action("ping", "连通性自检", false, "返回插件 id、扩展点与契约版本")
            .expect("静态构建")
            .build()
    }

    fn handle_event(ev: types::Event) -> Result<Vec<types::Effect>, String> {
        let event = ev.topic.clone();
        let payload = String::from_utf8_lossy(&ev.payload).to_string();
        host::log("debug", &format!("{} event={}", PLUGIN_ID, event));
        match event.as_str() {
            "skill.policy.prepare" => {
                let rid = checked_req_id(&payload)?;
                let v = payload_of(&payload)?;
                let requested = v
                    .get("requested_steps")
                    .and_then(Value::as_u64)
                    .unwrap_or(2);
                // 宿主要求 max_steps ∈1..=32 且 ≤ requested_steps。
                let max_steps = requested.clamp(1, 2).min(32);
                Ok(vec![result(
                    &json!({
                        "allow": true,
                        "max_steps": max_steps,
                        "route": "native",
                    }),
                    &rid,
                )])
            }
            // 会话观察者挂钩：接受但不干预，避免产生未知事件噪音。
            e if e.starts_with("hook.") => Ok(vec![]),
            other => Err(format!("未知事件：{other}")),
        }
    }

    fn handle_service(
        service: String,
        method: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        // 未声明服务的插件一律显式拒绝（宿主 call-service 会拿到这条消息）
        Err(format!(
            "{PLUGIN_ID} 未提供服务 {service}.{method}（{} 字节载荷）",
            payload.len()
        ))
    }

    fn handle_action(action_id: String, payload: String) -> Result<String, String> {
        match action_id.as_str() {
            "ping" => Ok(format!(
                "插件={} 扩展点=skill.policy 契约=0.1 参数={}",
                PLUGIN_ID, payload
            )),
            other => Err(format!("未知动作：{other}")),
        }
    }

    fn self_test() -> Result<Vec<types::TestResult>, String> {
        let rid = req_id_of(r#"{"req_id":"selftest"}"#);
        let parsed = payload_of(r#"{"req_id":"selftest","v":1}"#)?;
        Ok(vec![types::TestResult {
            name: "req-id-parse".to_string(),
            ok: rid == "selftest" && parsed.get("v").and_then(Value::as_i64) == Some(1),
            detail: format!("req_id={} 结果通道={}:<req_id>", rid, RESULT_PREFIX),
        }])
    }
}

xiaozhi_plugin_sdk::export!(Component);
