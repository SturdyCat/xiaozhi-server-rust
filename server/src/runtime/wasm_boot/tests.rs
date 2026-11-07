//! 生命周期引导回归：真实夹具 + 真实 SQLite 配置库 + 真实产物目录。
//! 覆盖：迁移路径（无标记 + 旧缺省启用 → 补注册+激活）、标记恢复 Registered、
//! 禁用插件停 Staged、依赖拓扑激活、缺依赖停 Registered、pin 不符跳过。

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::store::ConfigStore;
use crate::config::store::sqlite::SqliteConfigStore;
use crate::config::{Config, PluginEntryConfig};
use crate::plugins::wasm::WasmHost;
use crate::plugins::wasm::artifacts::ArtifactDir;
use crate::plugins::wasm::tests::{ensure_fixture, fast_limits};

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xz-wasm-boot-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

struct Rig {
    root: PathBuf,
    dir: ArtifactDir,
    host: WasmHost,
    store: Arc<SqliteConfigStore>,
    config: Config,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let root = temp_root(tag);
        let db = root.join("data.db");
        let mut config = Config::default();
        config.memory.db_path = root.join("memory.db").display().to_string();
        let store = SqliteConfigStore::open(db.to_str().expect("db path")).expect("store");
        store.persist(&config).expect("persist default");
        let dir =
            ArtifactDir::for_location(store.location()).expect("artifact dir from db location");
        Self {
            root,
            dir,
            host: WasmHost::new(fast_limits()).expect("host"),
            store,
            config,
        }
    }

    fn put_artifact(&self, id: &str, fixture_name: &str) {
        self.dir
            .save(id, &ensure_fixture(fixture_name))
            .expect("save artifact");
    }

    fn entry(&mut self, id: &str, enabled: bool) {
        self.config.plugins.entries.insert(
            id.to_string(),
            PluginEntryConfig {
                enabled,
                artifact: None,
                attestation: None,
            },
        );
    }

    fn boot(&mut self) {
        // 与 Engines::assemble 同序：先持久化当前配置，再把盘上态交给引导
        self.store.persist(&self.config).expect("persist config");
        crate::runtime::wasm_boot::boot_lifecycle(
            &self.config.plugins,
            &self.dir,
            &self.host,
            self.store.as_ref(),
        );
    }

    fn lifecycle(&self, id: &str) -> &'static str {
        self.host
            .lifecycle_of(id)
            .map(|(lc, _)| lc.as_str())
            .unwrap_or("missing")
    }

    fn stored_enabled(&self, id: &str) -> bool {
        self.store
            .load()
            .expect("load store")
            .plugins
            .entries
            .get(id)
            .map(|e| e.enabled)
            .unwrap_or(false)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn boot_migrates_legacy_enabled_plugin_without_marker() {
    let mut rig = Rig::new("migrate");
    // 老部署形态：产物存在、无注册标记、无条目（旧缺省启用语义）
    rig.put_artifact("hello.official", "hello");
    rig.boot();
    assert_eq!(
        rig.lifecycle("hello.official"),
        "active",
        "迁移应补注册并激活"
    );
    assert!(
        rig.dir.read_registered_marker("hello.official").is_some(),
        "迁移应写注册标记"
    );
    assert!(
        rig.stored_enabled("hello.official"),
        "迁移应固化 enabled=true"
    );
    // Active 投影：菜单与 UI 才出现
    let row = &rig.host.snapshot_json()["plugins"][0];
    assert_eq!(row["lifecycle"], "active");
    assert!(!row["registration"].is_null());
}

#[test]
fn boot_restores_marker_as_registered_without_activation() {
    let mut rig = Rig::new("marker");
    rig.put_artifact("hello.official", "hello");
    rig.dir
        .write_registered_marker("hello.official", "0.x")
        .expect("marker");
    rig.entry("hello.official", false);
    rig.boot();
    assert_eq!(
        rig.lifecycle("hello.official"),
        "registered",
        "标记恢复 Registered"
    );
    assert!(!rig.stored_enabled("hello.official"));
    assert!(rig.host.plugin_ui_json("hello.official").is_none());
    assert!(
        rig.host.snapshot_json()["plugins"][0]["registration"].is_null(),
        "Registered 不投影 registration"
    );
}

#[test]
fn boot_keeps_disabled_plugin_staged() {
    let mut rig = Rig::new("staged");
    rig.put_artifact("hello.official", "hello");
    rig.entry("hello.official", false);
    rig.boot();
    assert_eq!(
        rig.lifecycle("hello.official"),
        "staged",
        "禁用产物只登记不注册"
    );
    assert!(rig.dir.read_registered_marker("hello.official").is_none());
}

#[test]
fn boot_activates_in_dependency_topological_order() {
    let mut rig = Rig::new("topo");
    // 字典序 greeter.usermade < hello.official：迁移注册顺序先依赖者后，
    // 激活必须按拓扑（hello 先、greeter 后）
    rig.put_artifact("greeter.usermade", "greeter-user");
    rig.put_artifact("hello.official", "hello");
    rig.entry("greeter.usermade", true);
    rig.entry("hello.official", true);
    rig.boot();
    assert_eq!(rig.lifecycle("hello.official"), "active");
    assert_eq!(
        rig.lifecycle("greeter.usermade"),
        "active",
        "依赖就绪后应被拓扑激活"
    );
    // 双 Active 无首选 → 官方默认优先解析（greeter 用户实现可经 preference 切换）
    assert_eq!(
        rig.host.snapshot_json()["extensions"]["demo.greeter"]["state"],
        "resolved"
    );
}

#[test]
fn boot_leaves_missing_dependency_registered() {
    let mut rig = Rig::new("missing-dep");
    // 只装依赖者（greeter-user requires hello）：缺依赖 → 停在 Registered
    rig.put_artifact("greeter.usermade", "greeter-user");
    rig.entry("greeter.usermade", true);
    rig.boot();
    assert_eq!(rig.lifecycle("greeter.usermade"), "registered");
    assert!(rig.host.plugin_ui_json("greeter.usermade").is_none());
}

#[test]
fn boot_skips_artifact_with_mismatched_pin() {
    let mut rig = Rig::new("pin");
    rig.put_artifact("hello.official", "hello");
    rig.config.plugins.require_artifact_pin = true;
    rig.config.plugins.entries.insert(
        "hello.official".to_string(),
        PluginEntryConfig {
            enabled: true,
            artifact: Some("f".repeat(64)),
            attestation: None,
        },
    );
    rig.boot();
    assert_eq!(
        rig.lifecycle("hello.official"),
        "missing",
        "钉版不符不得登记"
    );
}

#[test]
fn boot_platform_gate_skips_everything() {
    let mut rig = Rig::new("gate");
    rig.put_artifact("hello.official", "hello");
    rig.config.plugins.enabled = false;
    rig.boot();
    assert_eq!(
        rig.lifecycle("hello.official"),
        "missing",
        "平台总闸关闭 = 零引导"
    );
}
