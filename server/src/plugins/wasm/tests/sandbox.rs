//! 沙箱陷阱测试（trap 夹具：fuel / epoch / 内存三重上限）。

use super::{fast_limits, fixture};
use crate::plugins::wasm::{SandboxLimits, WasmHost};

#[test]
fn fuel_exhaustion_marks_degraded() {
    let limits = SandboxLimits {
        call_fuel: 1_000_000,     // 1M fuel：很快耗尽
        call_epoch_ticks: 60_000, // 60s：让 fuel 先触发
        ..SandboxLimits::default()
    };
    let host = WasmHost::new(limits).expect("host");
    host.load("trap.hostile", &fixture("trap")).expect("load");
    let err = host.handle_event("trap.hostile", "spin", "").unwrap_err();
    let snap = host.snapshot_json();
    assert_eq!(snap["plugins"][0]["phase"]["state"], "degraded");
    assert_eq!(
        snap["plugins"][0]["phase"]["trap"], "Fuel",
        "陷阱分类（err={err}）"
    );
}

#[test]
fn epoch_deadline_marks_degraded() {
    let limits = SandboxLimits {
        call_fuel: u64::MAX,  // fuel 不设限
        call_epoch_ticks: 50, // 50ms 挂钟预算
        ..SandboxLimits::default()
    };
    let host = WasmHost::new(limits).expect("host");
    host.load("trap.hostile", &fixture("trap")).expect("load");
    let err = host.handle_event("trap.hostile", "spin", "").unwrap_err();
    let snap = host.snapshot_json();
    assert_eq!(snap["plugins"][0]["phase"]["state"], "degraded");
    assert_eq!(
        snap["plugins"][0]["phase"]["trap"], "Epoch",
        "陷阱分类（err={err}）"
    );
}

#[test]
fn memory_limit_rejects_oversized_component_at_instantiation() {
    // 实例化期拒绝：初始内存页（64KiB）超过上限 → 装载失败（不落表）
    let limits = SandboxLimits {
        max_memory_bytes: 16 * 1024, // 16KiB < 1 页
        ..SandboxLimits::default()
    };
    let host = WasmHost::new(limits).expect("host");
    let err = host.load("trap.hostile", &fixture("trap")).unwrap_err();
    let msg = format!("{err:#}"); // anyhow 链式展开：外层 context + 根因
    assert!(
        msg.to_lowercase().contains("memory"),
        "应报内存上限相关错误：{msg}"
    );
    assert_eq!(
        host.snapshot_json()["plugins"].as_array().map(Vec::len),
        Some(0)
    );
}

#[test]
fn memory_hog_degrades_within_limit() {
    // wasm 语义：memory.grow 被限流器拒绝时返回 -1（不陷阱）→ guest 分配器
    // 得到 null → Vec 分配失败 panic → 陷阱分类为 Trap（不是 Memory）。
    // 上限真正生效的证据：guest 内存被钉在 2MiB 封顶、宿主不随之膨胀。
    let limits = SandboxLimits {
        call_fuel: u64::MAX,
        call_epoch_ticks: 60_000,
        max_memory_bytes: 2 * 1024 * 1024,
        ..SandboxLimits::default()
    };
    let host = WasmHost::new(limits).expect("host");
    host.load("trap.hostile", &fixture("trap")).expect("load");
    let _err = host.handle_event("trap.hostile", "alloc", "").unwrap_err();
    let snap = host.snapshot_json();
    assert_eq!(snap["plugins"][0]["phase"]["state"], "degraded");
    assert_eq!(
        snap["plugins"][0]["phase"]["trap"], "Trap",
        "guest 侧 OOM panic 的真实形态（增长拒绝在 wasm 语义中不陷阱）"
    );
}

// ============================================================
