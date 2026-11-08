// wakeup.official：唤醒问候策略。
//
// 新架构下 server 只做系统底座，重构前的「唤醒问候策略」功能以插件形式提供。
// 执行模型遵循平台铁律：宿主派发事件 → 插件同步返回 Vec<Effect>（纯决策 + 效果）→
// 宿主过权限门后异步执行，并从插件隔离 kv 的 `<结果键>:<req_id>` 读回有界 JSON。
// 数据面（PCM、模型推理、设备协议、HTTP/SSE）一律留在原生宿主，插件不持有 I/O 句柄。

#[global_allocator]
static ALLOC: guest_alloc::GrowingBump = guest_alloc::GrowingBump::new();

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::builder::{FieldSpec, ManifestBuilder, RegistrationBuilder};
use xiaozhi_plugin_sdk::exports::lifecycle::Guest;
use xiaozhi_plugin_sdk::xiaozhi::plugin::{host, types};

/// 插件 id：必须与上传请求 id、manifest.toml 登记项完全一致。
const PLUGIN_ID: &str = "wakeup.official";
/// 结果通道前缀：宿主按 `<prefix>:<req_id>` 读回本插件的决策。
const RESULT_PREFIX: &str = "wakeup-result";

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
        ManifestBuilder::new(PLUGIN_ID, "唤醒问候策略")
            .expect("静态构建：id/title 已内联校验")
            .version("0.1.0")
            .description("wakeup.strategy 扩展点：从原生目录中选择问候语索引")
            .permission("kv")
            .expect("kv 权限")
            .implements("wakeup.strategy")
            .expect("扩展点")
            .official(true)
            .build()
            .expect("静态构建")
    }

    fn register() -> types::Registration {
        RegistrationBuilder::new()
            .menu_entry("nav.main", "wakeup-policy", "唤醒策略", "插件", 117, "settings")
            .expect("静态构建")
            .config_field(
                FieldSpec::int("greeting_index", "问候语索引")
                    .hint("0")
                    .help("原生目录下标")
                    .build(),
            )
            .expect("静态构建")
            .action("ping", "连通性自检", false, "返回插件 id、扩展点与契约版本")
            .expect("静态构建")
            .build()
    }

    fn handle_event(event: String, payload: String) -> Result<Vec<types::Effect>, String> {
        host::log("debug", &format!("{} event={}", PLUGIN_ID, event));
        match event.as_str() {
            "wakeup.prepare" => {
                let rid = checked_req_id(&payload)?;
                let v = payload_of(&payload)?;
                let count = v
                    .get("greetings")
                    .and_then(Value::as_array)
                    .map(|a| a.len())
                    .unwrap_or(0);
                if count == 0 {
                    return Err("问候目录为空，无法选择问候语".to_string());
                }
                // 只选目录下标，不注入任意文本/音频（宿主按索引取原生话术）。
                let index = 0usize.min(count - 1);
                Ok(vec![result(
                    &json!({"greeting_index": index, "route": "native"}),
                    &rid,
                )])
            }
            // 会话观察者挂钩：接受但不干预，避免产生未知事件噪音。
            e if e.starts_with("hook.") => Ok(vec![]),
            other => Err(format!("未知事件：{other}")),
        }
    }

    fn handle_action(action_id: String, payload: String) -> Result<String, String> {
        match action_id.as_str() {
            "ping" => Ok(format!(
                "插件={} 扩展点=wakeup.strategy 契约=0.1 参数={}",
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
