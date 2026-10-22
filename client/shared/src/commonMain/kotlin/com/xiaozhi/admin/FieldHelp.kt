package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.AlertDialog
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 字段帮助（ⓘ tooltip）的全局状态与渲染（作者：bigboss，2026-10-23）。
 *
 * ## 为什么是全局单例而不是每字段局部状态
 *
 * 配置页一张卡有几十个字段，每个字段各挂一个 AlertDialog 会把弹层节点数翻倍；
 * 全局只保留「当前打开的那一个」，badge 点击只写状态，页面根部渲染的 [fieldHelpDialog]
 * 统一消费——弹层节点恒为 1，显隐由 `visible` observable 驱动（Kuikly 响应式口径）。
 *
 * ## 交互口径（产品决策 2026-10：key 不上界面，中文名 + 按需解释）
 *
 * - 复杂字段：中文名旁挂 ⓘ（[infoBadge]），点开弹层看完整作用说明（原文案从
 *   常驻 caption 收进弹层，界面只留一行）；
 * - 见名知意的字段（CPU / AI 这类通用词、端口 / 语速等）：不挂 ⓘ；
 * - 弹层点击蒙层或「知道了」关闭；同一时刻只有一个弹层。
 *
 * ⚠️ Kuikly 响应式只跟踪构建期对 observable 的读取：badge 的 click 回调里**写**状态、
 * 对话框 attr/vif 闭包里**读**状态（见 AdminWidgets.dropdownField 同款约定）。
 */
object FieldHelpTip {
    /** 是否显示帮助弹层（observable：对话框显隐唯一真源）。 */
    var visible by observable(false)

    /** 弹层标题 = 字段中文名。 */
    var title by observable("")

    /** 弹层正文 = 字段作用详解（可多行）。 */
    var text by observable("")

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

/**
 * ⓘ 信息徽标：18×18 圆形描边 + 居中「i」，点击弹出该字段的帮助弹层。
 *
 * 只承载 UI：展示什么由调用方以 (title, help) 注入（复用组件不藏业务，见仓库复用组件铁律）。
 * 挂在字段标签行/开关行尾部；`help` 为空时调用方不应渲染本组件（vif 在调用方控制）。
 */
fun ViewContainer<*, *>.infoBadge(title: String, help: String) {
    View {
        attr {
            size(18f, 18f)
            marginLeft(6f)
            borderRadius(9f)
            border(Border(1f, BorderStyle.SOLID, AdminColors.textTertiary))
            alignItemsCenter()
            justifyContentCenter()
            // 可点击目标：悬停指针，提示可点
            cursor(CursorType.POINTER)
        }
        event {
            click { FieldHelpTip.show(title, help) }
        }
        Text {
            attr {
                fontSize(11f)
                fontWeightMedium()
                color(AdminColors.textTertiary)
                text("i")
            }
        }
    }
}

/**
 * 字段帮助弹层（每页根部渲染一次）：官方 AlertDialog（Modal 渲染，不占布局、
 * 蒙层点击关闭、自带过渡），暗色卡片 = 标题 + 可滚动正文 + 「知道了」。
 * 显隐/内容由 [FieldHelpTip] 全局状态驱动（见其文档的响应式约定）。
 */
fun ViewContainer<*, *>.fieldHelpDialog() {
    val modalWidth = {
        (getPager().pageData.pageViewWidth - 2 * AdminSpace.md).coerceAtMost(360f).coerceAtLeast(0f)
    }
    AlertDialog {
        attr {
            showAlert(FieldHelpTip.visible)
            inWindow(true)
            customContentView {
                View {
                    attr {
                        width(modalWidth())
                        backgroundColor(AdminColors.cardBg)
                        borderRadius(AdminShape.radiusLg)
                        border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
                        padding(AdminSpace.cardPadding)
                    }
                    Text {
                        attr {
                            fontSize(AdminType.section)
                            fontWeightMedium()
                            color(AdminColors.textPrimary)
                            marginBottom(AdminSpace.sm)
                            text(FieldHelpTip.title)
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
                                text(FieldHelpTip.text)
                            }
                        }
                    }
                    View { attr { height(AdminSpace.sm) } }
                    secondaryButton("知道了") { FieldHelpTip.dismiss() }
                }
            }
        }
        event {
            // 蒙层点击关闭（与 dropdownField 同一口径）
            clickBackgroundMask { FieldHelpTip.dismiss() }
        }
    }
}
