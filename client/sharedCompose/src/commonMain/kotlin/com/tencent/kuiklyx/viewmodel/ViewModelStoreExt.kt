package com.tencent.kuiklyx.viewmodel

import com.tencent.kuikly.core.base.ComposeView
import com.tencent.kuikly.core.log.KLog
import com.tencent.kuikly.core.pager.Pager

/**
 * ViewModel 委托函数
 * Copyright (c) 2025 Tencent Music Entertainment Group. All rights reserved.
 */
inline fun <reified VM : ViewModel> Pager.viewModelStore(
    // ViewModel 构造方法，提供 PagerId 给 ViewModel 的构造函数
    noinline factory: (pagerId: String) -> VM,
): Lazy<VM> = lazy(LazyThreadSafetyMode.NONE) {
    // 从 Store 中获取 ViewModel，不存在则创建并进行绑定
    ViewModelStore.get(this, VM::class) as? VM ?: factory(this.pagerId).also {
        ViewModelStore.setup(this, it)
    }
}

inline fun <reified VM : ViewModel> ComposeView<*, *>.viewModelStore(
    noinline factory: (pagerId: String) -> VM,
): Lazy<VM> = lazy(LazyThreadSafetyMode.NONE) {
    // 对于 View 级别的 ViewModel，需要先获取到当前 View 所属的 Pager
    (getPager() as? Pager)?.let { pager ->
        ViewModelStore.get(pager, VM::class) as? VM ?: factory(this.pagerId).also {
            ViewModelStore.setup(pager, it)
        }
    } ?: run {
        KLog.e(ViewModelStore.TAG, "failed get pager from view, check your view status")
        factory(pagerId)
    }
}
