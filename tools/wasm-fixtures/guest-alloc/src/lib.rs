//! wasm32-unknown-unknown 极简 bump 分配器：用 `memory.grow` 内联指令按需扩线性内存。
//!
//! **必须从 `__heap_base` 之后起分配**：线性内存低地址是 guest 静态数据段
//! （字符串字面量、静态量、shadow stack）；从 0/1 起 bump 会覆盖数据段
//! （实测第一笔分配就破坏分配器自身状态 → guest panic 陷阱）。
//! wit-bindgen 只为导出生成 cabi_realloc，不提供通用 GlobalAlloc；
//! guest 内 `format!`/`Vec` 需要 `#[global_allocator]`。不释放（bump），
//! 对测试夹具足够；宿主侧内存上限由 ResourceLimiter 兜底。

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

// LLD 为 wasm 提供的链接符号：静态数据段之后的第一个可用地址
unsafe extern "C" {
    static __heap_base: u8;
}

pub struct GrowingBump {
    /// 0 = 未初始化；首次分配时定位到 __heap_base
    head: AtomicUsize,
    lock: UnsafeCell<()>,
}

const WASM_PAGE: usize = 64 * 1024;

unsafe impl Sync for GrowingBump {}

impl GrowingBump {
    pub const fn new() -> Self {
        Self {
            head: AtomicUsize::new(0),
            lock: UnsafeCell::new(()),
        }
    }

    fn heap_start() -> usize {
        ptr::addr_of!(__heap_base) as usize
    }
}

unsafe impl GlobalAlloc for GrowingBump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // 单线程 bump：测试 guest 无并发
        let _guard = &*self.lock.get();
        let align = layout.align().max(1);
        let size = layout.size().max(1);
        let mut head = self.head.load(Ordering::Relaxed);
        if head == 0 {
            head = Self::heap_start().max(1);
        }
        let aligned = (head + align - 1) / align * align;
        let next = aligned.checked_add(size).unwrap_or(usize::MAX);
        let mem_size = wasm_mem_size();
        if next >= mem_size {
            let need_pages = (next - mem_size + WASM_PAGE) / WASM_PAGE;
            if wasm_memory_grow(need_pages).is_err() {
                return ptr::null_mut();
            }
        }
        self.head.store(next, Ordering::Relaxed);
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // bump：不释放（测试夹具语义）
    }
}

#[cfg(target_arch = "wasm32")]
fn wasm_mem_size() -> usize {
    core::arch::wasm32::memory_size(0) * WASM_PAGE
}

#[cfg(target_arch = "wasm32")]
fn wasm_memory_grow(pages: usize) -> Result<(), ()> {
    // memory_grow 返回旧页数（usize），失败为 usize::MAX
    let prev = core::arch::wasm32::memory_grow(0, pages);
    if prev == usize::MAX {
        Err(())
    } else {
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn wasm_mem_size() -> usize {
    usize::MAX
}

#[cfg(not(target_arch = "wasm32"))]
fn wasm_memory_grow(_pages: usize) -> Result<(), ()> {
    Err(())
}
