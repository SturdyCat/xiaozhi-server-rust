//! 运行期数据的可失败分配（分配失败 → 显式 [`Error::Alloc`]，不 panic 成陷阱）。
//!
//! wasm32 下 `memory.grow` 被 ResourceLimiter 拒绝时 Rust 分配器得到 null → 普通
//! `String`/`Vec` 操作直接 panic（宿主归类为 Trap → 插件 Degraded）。对**运行期载荷**
//! （事件 payload 拼接、kv 值、派生 JSON）这太重：一次大载荷就该是普通的业务失败。
//! 先 `try_reserve` 再写入，容量已保留则后续写入不再分配、不会失败。
//!
//! 静态注册数据**不在这里**：见 crate 文档「分配纪律的分界」——构建器走 expect 语义。

use std::collections::TryReserveError;

use crate::error::{Error, Result};

fn alloc_err(ctx: &str, e: TryReserveError) -> Error {
    Error::Alloc(format!("{ctx}（{e:?}）"))
}

/// 预留容量后克隆字符串（失败 = 显式 Alloc 错误，不 panic）。
pub fn try_string(src: &str, ctx: &str) -> Result<String> {
    let mut s = String::new();
    s.try_reserve_exact(src.len())
        .map_err(|e| alloc_err(ctx, e))?;
    s.push_str(src); // 容量已保留：不再分配
    Ok(s)
}

/// 预留容量的空字符串。
pub fn try_string_with_capacity(cap: usize, ctx: &str) -> Result<String> {
    let mut s = String::new();
    s.try_reserve_exact(cap).map_err(|e| alloc_err(ctx, e))?;
    Ok(s)
}

/// 预留容量的空向量。
pub fn try_vec_with_capacity<T>(cap: usize, ctx: &str) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(cap).map_err(|e| alloc_err(ctx, e))?;
    Ok(v)
}

/// 追加内容到已存在字符串；容量不足时先 try_reserve（失败 = 显式 Alloc 错误）。
pub fn try_push_str(dst: &mut String, src: &str, ctx: &str) -> Result<()> {
    if dst.capacity() - dst.len() < src.len() {
        dst.try_reserve_exact(src.len())
            .map_err(|e| alloc_err(ctx, e))?;
    }
    dst.push_str(src);
    Ok(())
}

/// 追加元素到已存在向量；容量不足时先 try_reserve（失败 = 显式 Alloc 错误）。
pub fn try_push<T>(dst: &mut Vec<T>, item: T, ctx: &str) -> Result<()> {
    if dst.capacity() == dst.len() {
        let more = dst.len().max(4);
        dst.try_reserve_exact(more).map_err(|e| alloc_err(ctx, e))?;
    }
    dst.push(item);
    Ok(())
}
