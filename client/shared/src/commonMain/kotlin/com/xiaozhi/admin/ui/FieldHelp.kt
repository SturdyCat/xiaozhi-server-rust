package com.xiaozhi.admin.ui

import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.AlertDialog
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 字段帮助（ⓘ tooltip）：状态 + 徽标 + 弹层（作者：bigboss，2026-10-23）。
 *
 * ## 交互口径（产品决策 2026-10：key 不上界面，中文名 + 按需解释）
 *
 * - 界面只出**中文见名知意**的字段名（CPU / AI 这类通用术语保留英文），配置 key 不上界面；
 * - 有复杂明细的字段：名称旁挂 ⓘ（[infoBadge]），点开弹层看完整说明（长段落从常驻 caption
 *   收进弹层，界面只留一行）；
 * - 见名知意的字段（端口 / 语速 / 声道数…）：**不挂** ⓘ——不是每个字段都需要解释；
 * - 弹层点击蒙层或「知道了」关闭；同一时刻只有一个弹层。
 *
 * ## 为什么状态挂在页面而不是全局单例
 *
 * observable 必须建在 `PagerScope` 上（全局 `by observable(...)` 已废弃，且脱离 Pager 的
 * 属性无法驱动任何页面重建）。所以状态由页面持有（[FieldHelpHost]），[infoBadge] 与
 * [fieldHelpDialog] 都用 `getPager()` 回溯到**同一个**页面实例——深埋在卡片里的徽标不必
 * 逐层传参，也不会出现"点了 A 页的 ⓘ、B 页弹层亮"的串页。
 *
 * 弹层节点也只为**当前页**渲染一次（挂在页面根部）：一张卡几十个字段若各挂一个
 * AlertDialog，节点数会翻倍。
 */
class FieldHelpState(scope: PagerScope) {
    /** 是否显示帮助弹层（observable：对话框显隐唯一真源）。 */
    var visible by scope.observable(false)

    /** 弹层标题 = 字段中文名。 */
    var title by scope.observable("")

    /** 弹层正文 = 字段作用详解（可多行）。 */
    var text by scope.observable("")

    /** badge 点击入口：一次写入三值，避免中间态闪烁。 */
    fun show(title: String, text: String) {
        this.title = title
        this.text = text
        visible = true
    }

    fun dismiss() {
        visible = false
    }
}

/** 页面提供字段帮助状态（[AdminPage] / [ConfigPage] 各持一份）。 */
interface FieldHelpHost {
    val fieldHelp: FieldHelpState
}

/**
 * 从任意容器回溯到所属页面的帮助状态。
 * 未实现 [FieldHelpHost] 的页面返回 null（该页不渲染 ⓘ，也不渲染弹层）。
 */
internal fun ViewContainer<*, *>.fieldHelpState(): FieldHelpState? = (getPager() as? FieldHelpHost)?.fieldHelp

/**
 * ⓘ 信息徽标：18×18 圆形描边 + 居中「i」，点击弹出该字段的帮助弹层。
 *
 * 只承载 UI：展示什么由调用方以 (title, help) 注入（复用组件不藏业务）。
 * 挂在字段标签行/开关行尾部；`help` 为空时调用方不应渲染本组件（[fieldLabel] 已控制）。
 */
fun ViewContainer<*, *>.infoBadge(title: String, help: String) {
    val state = fieldHelpState() ?: return
    View {
        attr {
            size(18f, 18f)
            marginLeft(6f)
            allCenter()
            // 可点击目标：悬停指针，提示可点
            cursor(CursorType.POINTER)
        }
        event {
            click { state.show(title, help) }
        }
        // 图标走官方 Canvas 矢量（不再用「i」字符 + 手绘圆形描边：字重与圆圈直径随字号漂移）
        appIcon({ IconToken.Info }, size = 16f, tint = { AdminColors.textTertiary })
    }
}

/**
 * 字段帮助弹层（每页根部渲染一次）：官方 AlertDialog（Modal 渲染，不占布局、
 * 蒙层点击关闭、自带过渡），暗色卡片 = 标题 + 可滚动正文 + 「知道了」。
 * 显隐/内容由本页的 [FieldHelpState] 驱动（见其文档的响应式约定）。
 */
fun ViewContainer<*, *>.fieldHelpDialog() {
    val state = fieldHelpState() ?: return
    val modalWidth = {
        (getPager().pageData.pageViewWidth - 2 * AdminSpace.md).coerceAtMost(360f).coerceAtLeast(0f)
    }
    AlertDialog {
        attr {
            showAlert(state.visible)
            inWindow(true)
            customContentView {
                // 弹层壳与卡片同一套外观（cardBg/圆角/1px 边框/cardPadding），宽度按窗口算
                dialogSurface(width = modalWidth) {
                    Text {
                        attr {
                            fontSize(AdminType.section)
                            fontWeightMedium()
                            color(AdminColors.textPrimary)
                            marginBottom(AdminSpace.sm)
                            text(state.title)
                        }
                    }
                    // 正文可能很长（整段字段说明收进弹层）：限高滚动，避免小屏溢出
                    Scroller {
                        attr {
                            height(300f)
                        }
                        Text {
                            attr {
                                fontSize(AdminType.body)
                                lineHeight(1.5f)
                                color(AdminColors.textSecondary)
                                text(state.text)
                            }
                        }
                    }
                    vGap(AdminSpace.sm)
                    secondaryButton("知道了") { state.dismiss() }
                }
            }
        }
        event {
            // 蒙层点击关闭（与 dropdownField 同一口径）
            clickBackgroundMask { state.dismiss() }
        }
    }
}
