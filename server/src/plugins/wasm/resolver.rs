//! 扩展点解析表：排他扩展点 → 唯一激活实现，用户插件失败自动回退官方默认。
//!
//! 语义（方案 v4 §二/§三）：
//!
//! - 每个排他扩展点（`asr.provider`、`connector.echomuse`…）解析到**唯一**实现；
//! - 首选 = 配置解析表指定的插件 id（用户替换官方默认的入口）；
//! - 首选不可用（未装载 / phase != Active / manifest 未声明该扩展点）→
//!   回退到声明了该扩展点且 `official=true` 的插件；再不行 → [`Resolution::Unresolved`]；
//! - 「官方默认不可卸载」由分发保证（随内核分发）；本表只做**确定性回退**。
//!
//! v1 无多层补丁优先级（DSH 的 overridden 层级）：解析表只有一层，
//! Overridden 语义在配置分层（P2 投影）落地时引入。

use std::collections::BTreeMap;

/// 扩展点解析结果
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// 解析到首选实现
    Resolved(String),
    /// 首选不可用，回退官方默认
    Fallback {
        official: String,
        preferred: String,
        reason: String,
    },
    /// 无可用实现（reason）
    Unresolved(String),
}

impl Resolution {
    /// 当前实际生效的实现 id（若有）
    pub fn active(&self) -> Option<&str> {
        match self {
            Resolution::Resolved(id) => Some(id),
            Resolution::Fallback { official, .. } => Some(official),
            Resolution::Unresolved(_) => None,
        }
    }
}

/// 解析时对插件世界的只读视图（避免 resolver 反向依赖 registry）。
pub trait PluginWorldView {
    /// 插件是否已装载且 phase == Active
    fn is_active(&self, plugin_id: &str) -> bool;
    /// 插件 manifest 是否声明实现该扩展点
    fn implements(&self, plugin_id: &str, point: &str) -> bool;
    /// 是否官方默认实现（manifest.official）
    fn is_official(&self, plugin_id: &str) -> bool;
    /// 声明实现该扩展点且激活的全部插件 id（官方默认发现用）
    fn implementors_of(&self, point: &str) -> Vec<String>;
}

/// 解析表（对应配置 `[extensions."<point>"] implementation = "<plugin-id>"`）。
/// 只存首选偏好；实际解析每次动态求值（phase 变化即时生效，无需失效通知）。
#[derive(Debug, Clone, Default)]
pub struct ResolutionTable {
    /// point -> 首选实现 id
    preferences: BTreeMap<String, String>,
}

impl ResolutionTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置/清除某扩展点的首选实现（None = 清除，回默认解析）
    pub fn set_preference(&mut self, point: &str, plugin_id: Option<String>) {
        match plugin_id {
            Some(id) => {
                self.preferences.insert(point.to_string(), id);
            }
            None => {
                self.preferences.remove(point);
            }
        }
    }

    pub fn preference(&self, point: &str) -> Option<&str> {
        self.preferences.get(point).map(String::as_str)
    }

    /// 解析一个排他扩展点。
    pub fn resolve(&self, point: &str, world: &dyn PluginWorldView) -> Resolution {
        if let Some(preferred) = self.preferences.get(point) {
            if world.is_active(preferred) && world.implements(preferred, point) {
                return Resolution::Resolved(preferred.clone());
            }
            let reason = if world.is_active(preferred) {
                format!("首选实现 {preferred} 的 manifest 未声明实现 {point}")
            } else {
                format!("首选实现 {preferred} 未装载或未激活")
            };
            if let Some(official) = pick_official(point, world) {
                return Resolution::Fallback {
                    official,
                    preferred: preferred.clone(),
                    reason,
                };
            }
            return Resolution::Unresolved(format!("{reason}，且无官方默认可回退"));
        }
        match pick_official(point, world) {
            // 无首选时官方默认即正选
            Some(official) => Resolution::Resolved(official),
            None => Resolution::Unresolved(format!(
                "扩展点 {point} 无可用实现（未配置首选，也无官方默认）"
            )),
        }
    }
}

/// 官方默认发现：激活实现里 official=true 的唯一者；
/// 多个官方默认属配置错误——显式暴露（管理台修复），不静默挑一个。
fn pick_official(point: &str, world: &dyn PluginWorldView) -> Option<String> {
    let officials: Vec<String> = world
        .implementors_of(point)
        .into_iter()
        .filter(|id| world.is_official(id))
        .collect();
    match officials.len() {
        1 => Some(officials.into_iter().next().expect("len==1")),
        0 => None,
        _ => {
            tracing::warn!(
                point,
                ?officials,
                "扩展点存在多个官方默认（配置错误），按注册序取第一个"
            );
            officials.into_iter().next()
        }
    }
}

/// 便捷视图：基于「id -> 条目」平面快照实现 [`PluginWorldView`]（测试与投影用）。
#[derive(Debug, Clone, Default)]
pub struct FlatWorld {
    pub plugins: BTreeMap<String, FlatEntry>,
}

#[derive(Debug, Clone)]
pub struct FlatEntry {
    pub active: bool,
    pub implements: Vec<String>,
    pub official: bool,
}

impl FlatWorld {
    pub fn insert(&mut self, id: &str, active: bool, implements: Vec<&str>, official: bool) {
        self.plugins.insert(
            id.to_string(),
            FlatEntry {
                active,
                implements: implements.into_iter().map(String::from).collect(),
                official,
            },
        );
    }
}

impl PluginWorldView for FlatWorld {
    fn is_active(&self, plugin_id: &str) -> bool {
        self.plugins
            .get(plugin_id)
            .map(|e| e.active)
            .unwrap_or(false)
    }

    fn implements(&self, plugin_id: &str, point: &str) -> bool {
        self.plugins
            .get(plugin_id)
            .map(|e| e.implements.iter().any(|p| p == point))
            .unwrap_or(false)
    }

    fn is_official(&self, plugin_id: &str) -> bool {
        self.plugins
            .get(plugin_id)
            .map(|e| e.official)
            .unwrap_or(false)
    }

    fn implementors_of(&self, point: &str) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|(_, e)| e.active && e.implements.iter().any(|p| p == point))
            .map(|(id, _)| id.clone())
            .collect()
    }
}
