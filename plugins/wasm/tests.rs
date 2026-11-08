//! WASM 插件平台测试（外置；夹具**按需本地构建，不入库**——用户方向 2026-11-04）。
//!
//! 夹具：`tests/fixtures/wasm/*.wasm`（真实 guest 组件；hello/trap/greeter-user 源在
//! tools/wasm-fixtures/，weather 源在 plugins-src/weather/——真实插件源根）。产物缺失或
//! 源已更新时自动重建（[`ensure_fixture`]）；构建工具缺失 → 显式失败而非跳过。

use super::effects::{enforce_permissions, Effect, Permissions};
use super::resolver::{FlatWorld, ResolutionTable};
use super::waterfall::{Participant, Strength, Waterfall, WaterfallOutcome};
use super::{contract, WasmHost};
use crate::plugins::wasm::sandbox::SandboxLimits;

pub(crate) fn fixture(name: &str) -> Vec<u8> {
    ensure_fixture(name)
}

/// 构建产物源集（任一更新即视为过期）：
/// 契约（WIT）+ guest SDK + 两个源根的全部 .rs/Cargo.toml/build.sh。
const SOURCE_ROOTS: [&str; 3] = [
    "tools/wasm-fixtures",
    "plugins-src/weather",
    "server/crates/xiaozhi-plugin-sdk",
];

fn dir_newest_mtime(
    dir: &std::path::Path,
    best: Option<std::time::SystemTime>,
) -> Option<std::time::SystemTime> {
    let mut best = best;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return best;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.is_dir() {
            if name == "target" {
                continue; // 构建输出不参与 staleness
            }
            best = dir_newest_mtime(&path, best);
        } else if name.ends_with(".rs")
            || name == "Cargo.toml"
            || name == "build.sh"
            || name == "plugin.wit"
        {
            if let Ok(m) = entry.metadata().and_then(|m| m.modified()) {
                best = Some(best.map_or(m, |b: std::time::SystemTime| b.max(m)));
            }
        }
    }
    best
}

fn artifact_stale(artifact: &std::path::Path, repo_root: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::metadata(artifact) else {
        return true;
    };
    let Ok(at) = meta.modified() else {
        return true;
    };
    let mut newest = dir_newest_mtime(&repo_root.join("wit"), None);
    for root in SOURCE_ROOTS {
        newest = dir_newest_mtime(&repo_root.join(root), newest);
    }
    match newest {
        Some(n) => n > at,
        None => false,
    }
}

/// 缺失或过期时本地重建全部夹具（本地实验产物，git 忽略），再读回字节。
/// 跨模块共享（gateway 端点测试也用）：`pub(crate)`。
pub(crate) fn ensure_fixture(name: &str) -> Vec<u8> {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.join(".."); // server/ → 仓库根
    let artifact = manifest_dir
        .join("tests/fixtures/wasm")
        .join(format!("{name}.wasm"));
    if artifact_stale(&artifact, &repo_root) {
        let scripts = [
            repo_root.join("tools/wasm-fixtures/build.sh"),
            repo_root.join("plugins-src/weather/build.sh"),
        ];
        for script in &scripts {
            let out = std::process::Command::new("bash")
                .arg(script)
                .current_dir(&repo_root)
                .output()
                .unwrap_or_else(|e| panic!("无法执行 {}（缺 bash？）：{e}", script.display()));
            if !out.status.success() {
                let tail = String::from_utf8_lossy(&out.stderr);
                let tail: String = tail
                    .lines()
                    .rev()
                    .take(12)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n");
                panic!(
                    "夹具构建失败 {}（wasm 产物不入库，需本地构建；前置：rustup target add wasm32-unknown-unknown + wasm-tools）：\n{tail}",
                    script.display()
                );
            }
        }
    }
    std::fs::read(&artifact).unwrap_or_else(|e| {
        panic!("夹具缺失 {artifact:?}（构建后仍缺，检查 build.sh 输出路径）：{e}")
    })
}

pub(crate) fn fast_limits() -> SandboxLimits {
    SandboxLimits::default()
}

/// 脚本化 fetcher：按序弹出应答并记录请求 URL（测试全程不出网）。
pub(crate) struct ScriptedFetcher {
    responses:
        std::sync::Mutex<std::collections::VecDeque<std::result::Result<(u16, String), String>>>,
    urls: std::sync::Mutex<Vec<String>>,
}

impl ScriptedFetcher {
    pub(crate) fn new() -> Self {
        Self {
            responses: std::sync::Mutex::new(std::collections::VecDeque::new()),
            urls: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn push_ok(&self, status: u16, body: &str) {
        self.responses
            .lock()
            .unwrap()
            .push_back(Ok((status, body.to_string())));
    }
    fn push_err(&self, msg: &str) {
        self.responses
            .lock()
            .unwrap()
            .push_back(Err(msg.to_string()));
    }
    fn fetch_count(&self) -> usize {
        self.urls.lock().unwrap().len()
    }
}

impl crate::plugins::wasm::effects::HttpFetcher for ScriptedFetcher {
    fn fetch(&self, url: &str) -> std::result::Result<(u16, String), String> {
        self.urls.lock().unwrap().push(url.to_string());
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err("脚本应答耗尽".into()))
    }
}

pub(crate) fn host_with(fetcher: &std::sync::Arc<ScriptedFetcher>) -> WasmHost {
    // ScriptedFetcher 自身实现 HttpFetcher：Arc 共享给宿主与测试两侧（各持强引用）
    WasmHost::with_fetcher(fast_limits(), fetcher.clone()).expect("host with fetcher")
}

// ============================================================
// hello 全链路
// ============================================================

#[test]
fn hello_loads_registers_and_resolves_extension() {
    let host = WasmHost::new(fast_limits()).expect("host");
    host.load("hello.official", &fixture("hello"))
        .expect("load");

    let snap = host.snapshot_json();
    assert_eq!(snap["plugins"][0]["id"], "hello.official");
    assert_eq!(snap["plugins"][0]["phase"]["state"], "active");
    assert_eq!(
        snap["plugins"][0]["registration"]["menu"][0]["slot"],
        "nav.main"
    );
    // 官方默认实现 → 扩展点直接解析到它
    assert_eq!(snap["extensions"]["demo.greeter"]["state"], "resolved");
    assert_eq!(
        snap["extensions"]["demo.greeter"]["implementation"],
        "hello.official"
    );
}

#[test]
fn hello_event_effects_execute_and_kv_persists() {
    let host = WasmHost::new(fast_limits()).expect("host");
    host.load("hello.official", &fixture("hello"))
        .expect("load");

    // kv 效果（声明了 kv 权限 → 放行并落 kv）
    host.handle_event("hello.official", "demo.kv", "你好世界")
        .expect("kv event");
    let stored = host.kv().get("hello.official", "last").expect("kv value");
    assert_eq!(stored, "你好世界".as_bytes());
    // 命名空间隔离：读不到裸键
    assert!(host.kv().get("hello.official", "").is_none());

    // echo 效果（log 免权限）
    let errs = host
        .handle_event("hello.official", "demo.echo", "回声")
        .expect("echo event");
    assert!(errs.is_empty(), "log 效果不应有执行错误：{errs:?}");

    // 插件业务性 Err（未知事件）不降级、不 panic
    host.handle_event("hello.official", "unknown.event", "")
        .expect("业务性失败不报宿主错误");
}

#[test]
fn hello_action_whitelist_and_greeting() {
    let host = WasmHost::new(fast_limits()).expect("host");
    host.load("hello.official", &fixture("hello"))
        .expect("load");

    let out = host
        .handle_action("hello.official", "greet", "小智")
        .expect("action");
    assert!(out.contains("你好"), "问候语应包含问候：{out}");

    // 未声明的动作直接拒绝（模型不能任意指定动作名）
    let err = host
        .handle_action("hello.official", "format-disk", "")
        .unwrap_err();
    assert!(
        err.to_string().contains("未声明动作"),
        "白名单拒绝文案：{err}"
    );
}

#[test]
fn hello_self_test_via_validate() {
    let host = WasmHost::new(fast_limits()).expect("host");
    let results = host
        .validate("hello.official", &fixture("hello"))
        .expect("validate");
    assert!(results.iter().all(|r| r.ok), "自检应全过：{results:?}");
    // validate 不落表
    let snap = host.snapshot_json();
    assert_eq!(snap["plugins"].as_array().map(Vec::len), Some(0));
}

#[test]
fn manifest_id_mismatch_rejected_and_old_instance_kept() {
    let host = WasmHost::new(fast_limits()).expect("host");
    host.load("hello.official", &fixture("hello"))
        .expect("load");
    // 用 hello 的组件冒充别的 id → 拒绝，旧实例保留
    let err = host.load("other.plugin", &fixture("hello")).unwrap_err();
    assert!(err.to_string().contains("不一致"), "id 不一致文案：{err}");
    assert_eq!(host.snapshot_json()["plugins"][0]["id"], "hello.official");
}

// ============================================================

mod architecture;
mod artifacts;
mod bridge;
mod extension;
mod http;
mod lan;
mod lifecycle;
mod sandbox;
mod ui;
mod weather;

// 契约校验 / 权限门 / waterfall（纯逻辑）
// ============================================================

#[test]
fn manifest_validation_rejects_bad_shapes() {
    let mut m = manifest_ok();
    assert!(contract::validate_manifest(&m).is_ok());

    m.id = "Bad ID".into();
    assert!(
        contract::validate_manifest(&m).is_err(),
        "大写/空格 id 应拒绝"
    );

    let mut m = manifest_ok();
    m.api_version = "9.9".into();
    assert!(
        contract::validate_manifest(&m).is_err(),
        "契约主版本不匹配应拒绝"
    );

    let mut m = manifest_ok();
    m.implements = vec!["no-dot".into()];
    assert!(contract::validate_manifest(&m).is_err(), "扩展点形状应校验");
}

fn manifest_ok() -> contract::ManifestDto {
    contract::ManifestDto {
        id: "demo.official".into(),
        version: "0.1.0".into(),
        api_version: "0.2".into(),
        title: "演示".into(),
        description: "desc".into(),
        permissions: vec![],
        implements: vec!["demo.greeter".into()],
        official: true,
        requires: vec![],
        subscriptions: vec![],
        endpoints: vec![],
        resources: Default::default(),
        config_namespace: "demo.official".into(),
        provides: vec![],
    }
}

#[test]
fn registration_validation_rejects_duplicates_and_malformed_slots() {
    let mut reg = contract::RegistrationDto::default();
    reg.menu.push(menu("nav.main", "a"));
    reg.menu.push(menu("nav.main", "a")); // 重复
    assert!(contract::validate_registration(&reg).is_err());

    // 槽位不设白名单（宿主不认识业务域）：只校验 `<域>.<名>` 形状
    let mut reg = contract::RegistrationDto::default();
    reg.menu.push(menu("sidebar.secret", "a"));
    assert!(
        contract::validate_registration(&reg).is_ok(),
        "形状合法的自定义槽位应被接受"
    );

    let mut reg = contract::RegistrationDto::default();
    reg.menu.push(menu("nodot", "a")); // 形状非法
    assert!(contract::validate_registration(&reg).is_err());
}

fn menu(slot: &str, id: &str) -> contract::MenuEntryDto {
    contract::MenuEntryDto {
        slot: slot.into(),
        id: id.into(),
        label: "标签".into(),
        group: "插件".into(),
        order: 1,
        icon: "settings".into(),
    }
}

#[test]
fn permission_gate_denies_undeclared_kv() {
    let no_kv = Permissions::parse(&[]).expect("empty perms");
    let denied = enforce_permissions(
        &no_kv,
        vec![Effect::KvPut {
            key: "k".into(),
            value: vec![1],
        }],
    );
    assert!(denied.is_err(), "未声明 kv 权限应拒绝");

    let with_kv = Permissions::parse(&["kv".into()]).expect("kv perms");
    assert!(enforce_permissions(
        &with_kv,
        vec![Effect::Log {
            level: "info".into(),
            message: "m".into(),
        }]
    )
    .is_ok());
}

#[test]
fn permissions_parse_rejects_unknown() {
    assert!(Permissions::parse(&["root".into()]).is_err());
    assert!(
        Permissions::parse(&["http-out:".into()]).is_err(),
        "空 host 应拒绝"
    );
}

#[test]
fn waterfall_runs_in_priority_order_with_trace() {
    let mut wf: Waterfall<i32> = Waterfall::new();
    // 注册：high 优先级 1 先执行，low 10 后执行
    wf.register(Participant {
        id: "low".into(),
        priority: 10,
        strength: Strength::Rewrite,
        handler: 0,
    })
    .unwrap();
    wf.register(Participant {
        id: "high".into(),
        priority: 1,
        strength: Strength::Observe,
        handler: 0,
    })
    .unwrap();
    let mut payload = 0;
    let (outcome, trace) = wf.run(&mut payload, |_, data| {
        *data += 1;
        Ok(false) // 全部透传
    });
    assert_eq!(outcome, WaterfallOutcome::Completed);
    assert_eq!(trace, vec!["high:next", "low:next"]);
    assert_eq!(payload, 2);

    // 重复注册即报错（对齐 DSH「重复即报错」）
    assert!(wf
        .register(Participant {
            id: "high".into(),
            priority: 1,
            strength: Strength::Observe,
            handler: 0
        })
        .is_err());

    // guard 否决短路
    let (outcome, trace) = wf.run(&mut payload, |p, _| {
        if p.id == "high" {
            Err("拒绝".into())
        } else {
            Ok(false)
        }
    });
    assert_eq!(outcome, WaterfallOutcome::Blocked("high".into()));
    assert_eq!(trace.len(), 1);
}
