// trap 作恶插件：事件名决定作恶方式（宿主沙箱陷阱测试的驱动方）。
// - "spin"：无限循环（黑盒防优化）→ fuel 或 epoch 陷阱（由宿主预算配置决定先触发谁）
// - "alloc"：无限分配 64KiB 块 → ResourceLimiter 内存陷阱

#[global_allocator]
static ALLOC: guest_alloc::GrowingBump = guest_alloc::GrowingBump::new();

wit_bindgen::generate!({
    path: "../../../server/wit",
    world: "plugin",
});

use crate::exports::lifecycle::Guest;
use crate::xiaozhi::plugin::types;

struct Component;

// 死循环（黑盒防优化）→ fuel/epoch 陷阱
fn evil_spin() -> ! {
    let mut i: u64 = 0;
    loop {
        i = i.wrapping_add(1);
        core::hint::black_box(&i);
    }
}

impl Guest for Component {
    fn manifest() -> types::PluginManifest {
        types::PluginManifest {
            id: "trap.hostile".to_string(),
            version: "0.1.0".to_string(),
            api_version: "0.1".to_string(),
            title: "作恶插件（测试）".to_string(),
            description: "沙箱陷阱夹具：无限循环/无限分配".to_string(),
            permissions: vec![],
            implements: vec![],
            official: false,
            requires: vec![],
        }
    }

    fn register() -> types::Registration {
        types::Registration {
            menu: vec![],
            config_fields: vec![],
            actions: vec![],
            ui_json: None,
        }
    }

    fn handle_event(event: String, _payload: String) -> Result<Vec<types::Effect>, String> {
        match event.as_str() {
            "spin" => evil_spin(),
            // 观察者挂钩路径的作恶面（P2d）：宿主以 hook.<position> 事件派发
            e if e.starts_with("hook.") => evil_spin(),
            "alloc" => {
                let mut keep: Vec<Vec<u8>> = Vec::new();
                loop {
                    keep.push(vec![0u8; 64 * 1024]);
                    core::hint::black_box(&keep);
                }
            }
            other => Err(format!("未知事件：{other}")),
        }
    }

    fn handle_action(_action_id: String, _payload: String) -> Result<String, String> {
        Err("不支持动作".to_string())
    }

    fn self_test() -> Result<Vec<types::TestResult>, String> {
        Ok(vec![types::TestResult {
            name: "noop".to_string(),
            ok: true,
            detail: "ok".to_string(),
        }])
    }
}

export!(Component);
