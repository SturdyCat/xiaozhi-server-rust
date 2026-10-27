package com.tencent.kuiklyx.viewmodel

import com.tencent.kuikly.core.base.PagerScope

/**
 * ViewModel
 * 支持关联到 Pager 生命周期
 * Copyright (c) 2025 Tencent Music Entertainment Group. All rights reserved.
 */
abstract class ViewModel : PagerScope {
    open fun onResumed() {}

    open fun onPaused() {}

    open fun onCleared() {}
}
