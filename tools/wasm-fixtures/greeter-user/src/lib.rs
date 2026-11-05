// greeter-user：用户自建兼容插件（官方默认 hello.official 的等价替代）。
//
// 演练「一切皆插件、任意可替换」的关键对照面：
// - 同一扩展点 demo.greeter，official=false → 扩展点解析表可首选它、失败回退官方；
// - 无任何权限声明 → kv-get 宿主导入被 deny-as-absent（返回 none + 宿主审计告警），
//   插件按「没有缓存」处理，而不是装载失败或运行期报错。

#[global_allocator]
static ALLOC: guest_alloc::GrowingBump = guest_alloc::GrowingBump::new();

use xiaozhi_plugin_sdk::builder::{ManifestBuilder, RegistrationBuilder};
use xiaozhi_plugin_sdk::exports::lifecycle::Guest;
use xiaozhi_plugin_sdk::xiaozhi::plugin::types;

struct Component;

impl Guest for Component {
    fn manifest() -> types::PluginManifest {
        ManifestBuilder::new("greeter.usermade", "问候插件（用户自建）")
            .expect("静态构建")
            .version("0.2.0")
            .description("demo.greeter 扩展点的用户实现：可替换官方默认，失败自动回退")
            // 刻意不声明任何权限（对照面：kv-get → deny-as-absent）
            .implements("demo.greeter")
            .expect("扩展点")
            .official(false)
            .build()
            .expect("静态构建")
    }

    fn register() -> types::Registration {
        RegistrationBuilder::new()
            .menu_entry(
                "nav.main",
                "user-greeter",
                "问候(用户)",
                "插件",
                101,
                "settings",
            )
            .expect("静态构建")
            .action("greet", "打招呼", false, "用户实现的问候")
            .expect("静态构建")
            .action(
                "recall",
                "回忆",
                false,
                "读 kv（无权限 → 宿主 deny-as-absent）",
            )
            .expect("静态构建")
            .build()
    }

    fn handle_event(event: String, _payload: String) -> Result<Vec<types::Effect>, String> {
        match event.as_str() {
            // 对照面：无 http-out 权限的效果 → 宿主权限门拒绝（Degraded，审计上报）
            "user.fetch" => Ok(vec![types::Effect::HttpGet(types::EffectHttpGet {
                request_id: "x".to_string(),
                url: "https://example.com/".to_string(),
            })]),
            other => Err(format!("用户插件不处理事件：{other}")),
        }
    }

    fn handle_action(action_id: String, payload: String) -> Result<String, String> {
        match action_id.as_str() {
            // 与官方默认可区分的输出（替换演练断言用）
            "greet" => Ok(format!("用户插件问候：{payload}！")),
            // 无 kv 权限：宿主返回 none，插件把它当「没有缓存」——这是拒绝的温和形态
            "recall" => Ok(
                match xiaozhi_plugin_sdk::xiaozhi::plugin::host::kv_get("last") {
                    Some(bytes) => format!(
                        "读到（不应发生：未声明 kv 权限）：{}",
                        String::from_utf8_lossy(&bytes)
                    ),
                    None => "无缓存（未声明 kv 权限，宿主 deny-as-absent）".to_string(),
                },
            ),
            other => Err(format!("未知动作：{other}")),
        }
    }

    fn self_test() -> Result<Vec<types::TestResult>, String> {
        Ok(vec![
            types::TestResult {
                name: "greet-shape".to_string(),
                ok: true,
                detail: "ok".to_string(),
            },
            types::TestResult {
                name: "kv-denied-as-absent".to_string(),
                ok: true,
                detail: "ok".to_string(),
            },
        ])
    }
}

xiaozhi_plugin_sdk::export!(Component);
