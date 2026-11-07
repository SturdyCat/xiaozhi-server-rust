//! 插件源根文档契约（用户要求固化为平台规则）：每个 `plugins-src/<插件>` 必须自带
//! `README.md` 且必含「功能描述」「使用方法」两节。纯结构检查，不依赖 wasm feature
//! ——文档是插件交付面，缺文档 = 新增插件未完成。

use std::path::Path;

#[test]
fn every_plugin_source_ships_usage_docs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins-src");
    let entries = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("插件源根 {} 必须存在：{e}", root.display()));
    let mut checked = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // 只检查插件目录：Cargo.toml 在 = 插件 crate；隐藏目录跳过
        if !dir.is_dir() || name.starts_with('.') || !dir.join("Cargo.toml").is_file() {
            continue;
        }
        let readme = dir.join("README.md");
        let content = std::fs::read_to_string(&readme).unwrap_or_else(|err| {
            panic!(
                "插件 {name} 缺少 README.md（功能描述与使用方法是交付面，见 plugins-src/AGENTS.md）：{err}"
            )
        });
        assert!(
            content.contains("## 功能"),
            "插件 {name} 的 README 需含 `## 功能` 节：{}",
            readme.display()
        );
        assert!(
            content.contains("## 使用"),
            "插件 {name} 的 README 需含 `## 使用` 节（事件/动作/配置/kv 键布局）：{}",
            readme.display()
        );
        checked.push(name);
    }
    assert!(
        !checked.is_empty(),
        "至少应检查到 weather 一个插件（目录扫描异常？）"
    );
}
