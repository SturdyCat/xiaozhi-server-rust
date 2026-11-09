//! wasm32-unknown-unknown guest 分配器：**可回收**（dlmalloc）。
//!
//! S3n 实测教训：此前的极简 bump 分配器只增不回收，插件长跑或高频事件
//! （每秒心跳 + 数据面帧）下堆持续增长，撞上沙箱 `max_memory_bytes`（64MB）
//! 后分配返回 null → wasm OOM trap → **整个 handle_event 中断、状态未落盘**
//! （表现为间歇性「事件处理无效果」，排查耗时极长的静默失败）。
//!
//! 现委托 [dlmalloc]（成熟实现：支持 free 回收、按页 `memory.grow`），
//! 类型名 [`GrowingBump`] 保留以免 15 个插件源码改动（`guest_alloc::GrowingBump::new()`）。
//!
//! [dlmalloc]: https://crates.io/crates/dlmalloc

use core::alloc::{GlobalAlloc, Layout};

use dlmalloc::GlobalDlmalloc;

/// guest 全局分配器（名称沿用历史，实现为可回收的 dlmalloc）。
pub struct GrowingBump;

impl GrowingBump {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for GrowingBump {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl GlobalAlloc for GrowingBump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { GlobalDlmalloc.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { GlobalDlmalloc.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { GlobalDlmalloc.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { GlobalDlmalloc.realloc(ptr, layout, new_size) }
    }
}
