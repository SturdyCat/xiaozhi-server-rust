//! 沙箱：三重上限（fuel / epoch / 内存）与陷阱分类。
//!
//! - **fuel**：每次调用确定性预算（wasm 指令数），防失控计算；
//! - **epoch**：挂钟超时兜底（需要独立线程按固定间隔递增引擎 epoch）；
//! - **内存**：`ResourceLimiter` 限制线性内存增长上限，防吃满宿主内存。
//!
//! 三者任一超限都以 wasm trap 形式浮出，[`classify_trap`] 把它归到 [`TrapKind`]，
//! 调用方（registry）据此置 `Degraded/Failed` 并触发扩展点回退，而不是让 panic 扩散。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::Result;
use wasmtime::{Engine, ResourceLimiter, Store};

/// 每插件的沙箱上限。默认值对应方案 v4 性能预算表（hook ≤5ms、动作 ≤5s）。
#[derive(Debug, Clone, Copy)]
pub struct SandboxLimits {
    /// 实例化 + manifest/register（激活期）的 fuel 预算
    pub activation_fuel: u64,
    /// 单次 handle-event / handle-action 调用的 fuel 预算
    pub call_fuel: u64,
    /// 单次调用的挂钟预算（epoch tick 数 = ms，见 [`EpochTicker::TICK`]）
    pub call_epoch_ticks: u64,
    /// 激活期挂钟预算
    pub activation_epoch_ticks: u64,
    /// 线性内存上限（字节）
    pub max_memory_bytes: usize,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        // epoch tick = 1ms（见 EpochTicker）。hook 类调用 5ms 档；动作给到 5s。
        // fuel 经验起点 1e8（cranelift 下约 5-20ms 量级的纯计算），P4 语音链路实测后校准。
        Self {
            activation_fuel: 2_000_000_000,
            call_fuel: 100_000_000,
            call_epoch_ticks: 5_000,
            activation_epoch_ticks: 5_000,
            max_memory_bytes: 64 * 1024 * 1024,
        }
    }
}

/// 陷阱分类：决定 phase 迁移与回退动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapKind {
    /// fuel 耗尽（失控计算）
    Fuel,
    /// epoch 超时（挂死/长循环）
    Epoch,
    /// 内存超限（ResourceLimiter 拒绝增长）
    Memory,
    /// wasm trap（除零、unreachable、访存越界等）
    Trap,
    /// 组件卸载后调用
    Unmounted,
}

/// epoch 递增线程：wasmtime 的 epoch 机制需要外部线程推进时钟。
///
/// 每 [`TICK`](Self::TICK) 递增一次；持有句柄.Drop 时停止并回收线程。
pub struct EpochTicker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    /// tick 粒度 = 1ms：`set_epoch_deadline(n)` 即「n 毫秒预算」。
    pub const TICK: Duration = Duration::from_millis(1);

    pub fn start(engine: &Engine) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let engine = engine.clone();
        let handle = std::thread::Builder::new()
            .name("wasm-epoch-ticker".into())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    engine.increment_epoch();
                    std::thread::sleep(Self::TICK);
                }
            })
            .expect("spawn epoch ticker");
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// 构建启用 epoch 中断与 fuel 的引擎（全局唯一，所有插件共享）。
pub fn build_engine() -> Result<Engine> {
    let mut cfg = wasmtime::Config::new();
    cfg.epoch_interruption(true);
    cfg.consume_fuel(true);
    Ok(Engine::new(&cfg)?)
}

// ============================================================
// 内存限制器
// ============================================================

/// Store 数据侧的内存限制状态（每插件独立上限）。
pub struct MemoryBudget {
    max_bytes: usize,
}

impl MemoryBudget {
    pub fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }
}

impl ResourceLimiter for MemoryBudget {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> std::result::Result<bool, wasmtime::Error> {
        Ok(desired <= self.max_bytes)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> std::result::Result<bool, wasmtime::Error> {
        // 表只承载少量句柄，限制内存已足够；放行但给个硬顶防异常表
        Ok(desired <= 1_000_000)
    }
}

// ============================================================
// 调用预算与陷阱分类
// ============================================================

/// 调用前给 Store 重置预算：fuel + epoch deadline。
/// `set_epoch_deadline` 语义是「相对当前 epoch 的刻度数」（tick=1ms，见 [`EpochTicker`]）。
pub fn arm_call_budget<T: 'static>(
    store: &mut Store<T>,
    fuel: u64,
    epoch_ticks: u64,
) -> Result<()> {
    store.set_fuel(fuel)?;
    store.set_epoch_deadline(epoch_ticks);
    Ok(())
}

/// 把调用错误归类：先解类型化 `wasmtime::Trap`（instance::call_err 用 msg 装箱
/// 保留类型），再按链上消息特征兜底（内存限制拒绝等非 trap 形态）。
pub fn classify_trap(err: &anyhow::Error) -> TrapKind {
    if let Some(we) = err.downcast_ref::<wasmtime::Error>() {
        if let Some(t) = we.downcast_ref::<wasmtime::Trap>() {
            return trap_kind_of(*t);
        }
        for cause in we.chain() {
            if let Some(t) = cause.downcast_ref::<wasmtime::Trap>() {
                return trap_kind_of(*t);
            }
            if msg_matches_memory(&cause.to_string()) {
                return TrapKind::Memory;
            }
        }
    }
    for cause in err.chain() {
        let msg = cause.to_string().to_ascii_lowercase();
        if msg.contains("fuel") {
            return TrapKind::Fuel;
        }
        if msg.contains("epoch") || msg.contains("interrupt") {
            return TrapKind::Epoch;
        }
        if msg_matches_memory(&msg) {
            return TrapKind::Memory;
        }
    }
    TrapKind::Trap
}

fn trap_kind_of(t: wasmtime::Trap) -> TrapKind {
    use wasmtime::Trap as T;
    match t {
        T::OutOfFuel => TrapKind::Fuel,
        T::Interrupt => TrapKind::Epoch,
        T::MemoryOutOfBounds | T::AllocationTooLarge => TrapKind::Trap,
        _ => TrapKind::Trap,
    }
}

fn msg_matches_memory(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("memory limit") || m.contains("exceeds memory limits") || m.contains("out of memory")
}
