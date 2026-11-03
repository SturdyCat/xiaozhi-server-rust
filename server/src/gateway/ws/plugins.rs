//! 能力清单与配置元数据端点（`docs/plugin-architecture-unification.md` §3.4）。
//!
//! - `GET /api/plugins`：能力 + 实现 + **实际状态**（`enabled` 与 `phase` 分离，
//!   并带 `last_error`）。此前管理页只能靠"保存后能不能用"猜后端，运行期失败全部静默。
//! - `GET /api/config/schema`：字段元数据（标签/类型/默认值提示/热生效语义/是否密钥）。
//!
//! ⚠️ 两者都是**只读投影**：不改变任何运行期行为（P1 的契约），也不做自动表单布局
//! （见 `docs/plugin-architecture-unification.md` §11.1：DSH 的 `autoGenerate` 至今没有
//! 客户端使用，我们只借"服务端作为字段唯一权威"这一点）。

use std::sync::Arc;

use axum::{extract::State, response::Response};

use super::json_ok;
use crate::gateway::ws::config::config_is_persistent;
use crate::plugins::registry;
use crate::runtime::Engines;

/// `GET /api/plugins`：能力清单 + 状态。
pub(super) async fn list_plugins(State(engines): State<Arc<Engines>>) -> Response {
    let mut body = engines.host.capabilities_json();
    body["ok"] = serde_json::json!(true);
    // 设备插件注册投影：管理台「设备管理」菜单的门控（见 [device_plugins_json]）
    body["device_plugins"] = device_plugins_json(&engines.config);
    // 配置来源一并带上：状态解读常需知道"这些值来自哪个文件、重启会不会丢"
    let config_path = engines.config_path();
    body["config"] = serde_json::json!({
        "path": config_path,
        "persistent": config_path.map(config_is_persistent),
    });
    json_ok(body)
}

/// 设备插件（echomuse / tuya）的**注册投影**——它们不是语音引擎（无 implementations），
/// 不进 `capabilities` 折叠，单独成数组随同一端点下发。
///
/// 注册语义 = 配置段启用（两段默认 `enabled=true`，即默认注册）：
/// - `registered=false`（用户显式关闭）→ 管理台隐藏「设备管理」里该插件的设备分区与
///   齿轮配置块；两个插件都未注册时整个「设备管理」菜单隐藏（注册后菜单才出现）；
/// - `phase` 只报 active/disabled 两态（纯配置投影，不做运行探测）；
/// - `note` 写热生效语义（echomuse RestartOnly / tuya 全热生效），与注册表同源结论。
///
/// ⚠️ 只读投影：不改变任何运行期行为（与本文件其余端点同一契约）。
// pub(super)：plugins_tests（兄弟模块）直驱纯函数做契约回归（与 list_plugins 同可见性口径）
pub(super) fn device_plugins_json(cfg: &crate::config::Config) -> serde_json::Value {
    let echomuse = cfg.echomuse.enabled;
    let tuya = cfg.tuya.enabled;
    let row = |id: &str, label: &str, registered: bool, section: &str, note: &str| {
        serde_json::json!({
            "id": id,
            "label": label,
            "registered": registered,
            "phase": if registered { "active" } else { "disabled" },
            "config_section": section,
            "note": note,
            "last_error": "",
        })
    };
    serde_json::json!([
        row(
            "echomuse",
            "EchoMuse 设备",
            echomuse,
            "echomuse",
            "RestartOnly：开关保存后需重启服务端生效",
        ),
        row(
            "tuya",
            "涂鸦设备",
            tuya,
            "tuya",
            "全部热生效：保存即生效，无需重启"
        ),
    ])
}

/// `GET /api/config/schema`：字段元数据（按能力聚合）。
pub(super) async fn config_schema(State(_engines): State<Arc<Engines>>) -> Response {
    let mut body = registry::schema_json();
    body["ok"] = serde_json::json!(true);
    json_ok(body)
}
