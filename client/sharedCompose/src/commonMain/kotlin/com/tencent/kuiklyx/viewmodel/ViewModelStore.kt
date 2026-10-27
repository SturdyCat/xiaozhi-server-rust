package com.tencent.kuiklyx.viewmodel

import com.tencent.kuikly.core.base.event.Event
import com.tencent.kuikly.core.log.KLog
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.IPagerEventObserver
import com.tencent.kuikly.core.pager.Pager
import kotlin.reflect.KClass

/**
 * ViewModelStore
 * Copyright (c) 2025 Tencent Music Entertainment Group. All rights reserved.
 */
object ViewModelStore {
    const val TAG = "ViewModelStore"

    private val store = mutableMapOf<Key, ViewModel>()
    private val observerMap = mutableMapOf<Pager, ViewModelLifeCycleObserver>()

    internal class ViewModelLifeCycleObserver(private val pager: Pager) : Event(),
        IPagerEventObserver {
        override fun onPagerEvent(pagerEvent: String, eventData: JSONObject) {
            when (pagerEvent) {
                Pager.PAGER_EVENT_DID_APPEAR -> {
                    store.filterKeys { it.isSamePager(pager) }.forEach {
                        it.value.onResumed()
                    }
                }

                Pager.PAGER_EVENT_DID_DISAPPEAR -> {
                    store.filterKeys { it.isSamePager(pager) }.forEach {
                        it.value.onPaused()
                    }
                }
            }
        }

        override fun onViewDidRemove() {
            store.filterKeys { it.isSamePager(pager) }.forEach {
                it.value.onCleared()
                store.remove(it.key)
            }
            observerMap.remove(pager)
            super.onViewDidRemove()
        }
    }

    /**
     * 根据 Key 获取 ViewModel
     */
    fun get(pager: Pager, vmClass: KClass<out ViewModel>): ViewModel? {
        return store[Key(pager, vmClass)]
    }

    /**
     * ViewModel 绑定 Pager
     * 1. 注册 Pager 对应生命周期的监听
     * 2. 按照 Pager 和类型保存 ViewModel
     */
    fun setup(pager: Pager, viewModel: ViewModel) {
        // 先判断是否首次为该 pager 注册 observer
        // 注意：getLifeCycleObserver() 内部使用 observerMap.getOrPut()，
        // 一旦调用就会 put，之后再 containsKey 一定返回 true。
        // 因此判断必须放在 getLifeCycleObserver() 之前。
        val isFirstRegister = !observerMap.containsKey(pager)

        // 获取或创建生命周期观察者
        val observer = getLifeCycleObserver(pager)

        // 只有首次注册才挂载事件监听器
        if (isFirstRegister) {
            // Pager 接口，监听页面可见性，对应 onResume 和 onPause
            pager.addPagerEventObserver(observer)
            // Event 接口，监听页面 View 销毁事件，对应 onDestroy
            pager.getViewEvent().putPluginEvent("$TAG#${pager.pagerId}", observer)

            KLog.d(TAG, "Registered lifecycle observer for pager: ${pager.pagerId}")
        }
        store[Key(pager, viewModel::class)] = viewModel
    }

    /**
     * 清空所有已注册的 ViewModel
     */
    fun clear() {
        // 清理所有 ViewModel
        store.values.forEach { viewModel ->
            viewModel.onCleared()
        }
        store.clear()

        // 清理所有观察者
        observerMap.clear()
    }

    private fun getLifeCycleObserver(pager: Pager): ViewModelLifeCycleObserver {
        return observerMap.getOrPut(pager) {
            ViewModelLifeCycleObserver(pager)
        }
    }

    internal class Key(private val pager: Pager, private val vmClass: KClass<out ViewModel>) {
        fun isSamePager(pager: Pager): Boolean {
            return this.pager == pager
        }

        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (other == null || this::class != other::class) return false
            other as Key
            return pager == other.pager && vmClass == other.vmClass
        }

        override fun hashCode(): Int {
            return 31 * pager.hashCode() + vmClass.hashCode()
        }

        override fun toString(): String {
            return "Key(pager=${pager.pagerId}, vmClass=${vmClass.simpleName}, hash=${hashCode()})"
        }
    }
}
