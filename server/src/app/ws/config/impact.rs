//! 一次保存的**生效档位**计算（`POST /api/config` 响应的 `hot` 字段）。
//!
//! 拆出原因：`AGENTS.md` §5.10 文件规模约定（父模块 `config.rs` 已接近上限），
//! 同时这块逻辑自成一体——"对比新旧配置 → 查注册表字段元数据 → 取最严格档位"。
//!
//! 为什么由服务端算（`docs/plugin-architecture-unification.md` §5.4 第 6 条）：
//! 字段级热生效语义的**唯一权威**是注册表（[`crate::plugins::registry`]）；让客户端
//! 自己猜会退化成"文档说立即生效、实际要重启"那类误导（§1.7 bug-2 的形态）。
//! 客户端只负责把 `hot` 翻成一句 toast 文案。

use crate::config::Config;
use crate::plugins::registry::{self, HotReload};

/// 一次保存的**实际生效档位**：变更字段里最严格的 `hot` + 变更段与字段路径。
pub(crate) struct SaveImpact {
    pub hot: HotReload,
    /// 变更的顶层配置段（去重、字典序，便于稳定断言与展示）。
    pub sections: Vec<String>,
    /// 变更字段的完整路径（点分；仅供日志/测试，不承诺 wire 稳定）。
    pub paths: Vec<String>,
}

/// 递归收集两次配置之间的**叶子级**变更路径（对象递归；数组与非对象整体算一个变更点）。
fn changed_paths(
    before: &serde_json::Value,
    after: &serde_json::Value,
    prefix: &mut Vec<String>,
    out: &mut Vec<Vec<String>>,
) {
    match (before.as_object(), after.as_object()) {
        (Some(a), Some(b)) => {
            // BTreeSet：键序稳定（同一份配置永远给出同样的 paths 顺序）
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for k in keys {
                prefix.push(k.clone());
                match (a.get(k), b.get(k)) {
                    (Some(x), Some(y)) => changed_paths(x, y, prefix, out),
                    _ => out.push(prefix.clone()),
                }
                prefix.pop();
            }
        }
        _ => {
            if before != after && !prefix.is_empty() {
                out.push(prefix.clone());
            }
        }
    }
}

/// 计算本次保存的生效档位：把所有变更字段的 `hot` 取**最严格**（见 [`HotReload::strictest`]）。
///
/// 逐级回退：字段完整路径 → 逐段缩短 → 配置段级（`registry::section_hot`，未登记段保守按需重启）。
pub(crate) fn save_impact(before: &Config, after: &Config) -> SaveImpact {
    // 两侧都先规范化：否则"旧位置字段被搬到规范位置"会被当成一次真实变更，
    // 把纯迁移误报成"需要重启"。
    let norm = |c: &Config| {
        let mut c = c.clone();
        c.normalize();
        serde_json::to_value(&c).unwrap_or(serde_json::Value::Null)
    };
    let (bv, av) = (norm(before), norm(after));
    let mut raw: Vec<Vec<String>> = Vec::new();
    changed_paths(&bv, &av, &mut Vec::new(), &mut raw);

    let mut hot: Option<HotReload> = None;
    for p in &raw {
        let refs: Vec<&str> = p.iter().map(|s| s.as_str()).collect();
        let h = (1..=refs.len())
            .rev()
            .find_map(|n| registry::field_hot(&refs[..n]))
            .unwrap_or_else(|| registry::section_hot(refs.first().copied().unwrap_or("")));
        hot = Some(hot.map_or(h, |acc| acc.strictest(h)));
    }
    let mut sections: Vec<String> = raw.iter().filter_map(|p| p.first().cloned()).collect();
    sections.sort();
    sections.dedup();
    let paths = raw.iter().map(|p| p.join(".")).collect();
    SaveImpact {
        // 无变更 → Live（"已是最新"，不该吓唬用户说需要重启）
        hot: hot.unwrap_or(HotReload::Live),
        sections,
        paths,
    }
}

