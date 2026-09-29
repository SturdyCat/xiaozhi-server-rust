package com.xiaozhi.admin

import android.app.Activity
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import androidx.appcompat.app.AppCompatActivity
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.render.android.IKRLogAdapter
import com.tencent.kuikly.core.render.android.IKRRouterAdapter
import com.tencent.kuikly.core.render.android.IKRThreadAdapter
import com.tencent.kuikly.core.render.android.KuiklyRenderAdapterManager
import com.tencent.kuikly.core.render.android.KuiklyRenderViewBaseDelegator
import com.tencent.kuikly.core.render.android.KuiklyRenderViewDelegatorDelegate
import com.tencent.kuikly.core.render.android.adapter.KRUncaughtExceptionHandlerAdapter
import java.util.concurrent.Executors

/**
 * Kuikly 页面承载容器（Activity 方式）。
 * 通过 pageName 打开 shared 中 @Page 注解对应的页面。
 */
class KuiklyRenderActivity : AppCompatActivity(), KuiklyRenderViewDelegatorDelegate {

    private lateinit var hrContainerView: ViewGroup
    private lateinit var kuiklyRenderViewDelegator: KuiklyRenderViewBaseDelegator

    private val pageName: String
        get() {
            val pn = intent.getStringExtra(KEY_PAGE_NAME) ?: ""
            return if (pn.isNotEmpty()) pn else "router"
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        kuiklyRenderViewDelegator = KuiklyRenderViewBaseDelegator(this, this)
        setContentView(R.layout.activity_hr)
        setupImmersiveMode()

        hrContainerView = findViewById(R.id.hr_container)
        kuiklyRenderViewDelegator.onAttach("", pageName, createPageData())
    }

    override fun onResume() {
        super.onResume()
        kuiklyRenderViewDelegator.onResume()
    }

    override fun onPause() {
        super.onPause()
        kuiklyRenderViewDelegator.onPause()
    }

    override fun onDestroy() {
        super.onDestroy()
        kuiklyRenderViewDelegator.onDetach()
    }

    private fun createPageData(): Map<String, Any> {
        val param = argsToMap()
        param["appId"] = 1
        return param
    }

    private fun argsToMap(): MutableMap<String, Any> {
        val jsonStr = intent.getStringExtra(KEY_PAGE_DATA) ?: return mutableMapOf()
        return JSONObject(jsonStr).toMap()
    }

    private fun setupImmersiveMode() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
            window.addFlags(WindowManager.LayoutParams.FLAG_DRAWS_SYSTEM_BAR_BACKGROUNDS)
            window.statusBarColor = Color.TRANSPARENT
            window.decorView.systemUiVisibility =
                View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN or View.SYSTEM_UI_FLAG_LAYOUT_STABLE
        }
    }

    companion object {
        private const val KEY_PAGE_NAME = "pageName"
        private const val KEY_PAGE_DATA = "pageData"

        init {
            // 初始化 Kuikly 适配器（图片 / 日志 / 路由 / 线程 / 异常）
            with(KuiklyRenderAdapterManager) {
                krImageAdapter = KRImageAdapter
                krLogAdapter = KRLogAdapter
                krRouterAdapter = KRRouterAdapter
                krThreadAdapter = KRThreadAdapter()
                krUncaughtExceptionHandlerAdapter = KRUncaughtExceptionHandlerAdapter
            }
        }

        fun start(context: android.content.Context, pageName: String, pageData: JSONObject) {
            val starter = android.content.Intent(context, KuiklyRenderActivity::class.java)
            starter.putExtra(KEY_PAGE_NAME, pageName)
            starter.putExtra(KEY_PAGE_DATA, pageData.toString())
            context.startActivity(starter)
        }
    }
}

/** 图片加载适配器（宿主必须实现，这里给出最简占位，替换为真实图片库） */
object KRImageAdapter : com.tencent.kuikly.core.render.android.adapter.KRImageAdapter {
    override fun getBitmap(
        context: android.content.Context,
        view: View,
        url: String,
        citizenId: String,
        callback: com.tencent.kuikly.core.render.android.adapter.KRImageAdapterCallback,
    ) {
        // TODO: 接入 Coil / Glide / 自研下载解码
    }
}

/** 日志适配器 */
object KRLogAdapter : IKRLogAdapter {
    override fun i(tag: String, msg: String) = android.util.Log.i(tag, msg)
    override fun d(tag: String, msg: String) = android.util.Log.d(tag, msg)
    override fun e(tag: String, msg: String) = android.util.Log.e(tag, msg)
}

/** 页面路由适配器 */
object KRRouterAdapter : IKRRouterAdapter {
    override fun openPage(context: android.content.Context, pageName: String, pageData: JSONObject) {
        if (context is Activity) {
            KuiklyRenderActivity.start(context, pageName, pageData)
        }
    }

    override fun closePage(context: android.content.Context) {
        (context as? Activity)?.finish()
    }
}

/** 线程适配器：复用宿主子线程池 */
class KRThreadAdapter : IKRThreadAdapter {
    override fun executeOnSubThread(task: () -> Unit) = execOnSubThread(task)
}

private val subThreadPoolExecutor by lazy { Executors.newFixedThreadPool(2) }
private fun execOnSubThread(runnable: () -> Unit) = subThreadPoolExecutor.execute(runnable)
