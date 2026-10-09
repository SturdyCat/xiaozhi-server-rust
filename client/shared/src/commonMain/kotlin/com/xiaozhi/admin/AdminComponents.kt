package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.base.ViewRef
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.collection.ObservableList
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.AlertDialog
import com.tencent.kuikly.core.views.Input
import com.tencent.kuikly.core.views.PageList
import com.tencent.kuikly.core.views.PageListView
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.ScrollParams
import com.tencent.kuikly.core.views.Switch
import com.tencent.kuikly.core.views.TabItem
import com.tencent.kuikly.core.views.Tabs
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.TextArea
import com.tencent.kuikly.core.views.View
import com.tencent.kuikly.core.views.compose.Button


// ===================== 基础排版 =====================

/** 分组标题：section(21)/500，textPrimary。 */
fun ViewContainer<*, *>.sectionTitle(title: String) {
    Text {
        attr {
            fontSize(AdminType.section)
            fontWeightMedium()
            color(AdminColors.textPrimary)
            marginBottom(AdminSpace.sm)
            text(title)
        }
    }
}

/** 卡片内 1px 横向分隔线（行间分隔）。 */
fun ViewContainer<*, *>.dividerH() {
    View {
        attr {
            height(1f)
            backgroundColor(AdminColors.divider)
            marginBottom(AdminSpace.sm)
        }
    }
}

/**
 * 次级说明文字（表单 hint / 就地提示）：caption(15) 或 micro(13) 字号 + 语义色。
 *
 * tone：`tertiary`（默认，解释"这个字段是干什么的"）/
 * `warn`（能构建但能力受限，如"未配 embedding → 词法模式"）/
 * `error`（不可用/失败，如加载失败 + 怎么修）。
 * 全站提示文字的**唯一**入口，避免每处各写一套字号与色值。
 */
fun ViewContainer<*, *>.textNote(text: String, tone: String = "tertiary", micro: Boolean = false) {
    Text {
        attr {
            fontSize(if (micro) AdminType.micro else AdminType.caption)
            color(
                when (tone) {
                    "warn" -> AdminColors.warningTintText
                    "error" -> AdminColors.dangerTintText
                    else -> AdminColors.textTertiary
                },
            )
            text(text)
        }
    }
}

// ===================== 卡片 =====================

/**
 * 分组卡片：cardBg、radiusLg(12)、1px divider 边框、内边距 cardPadding(20)。
 * 不带左右外边距——页面留白由 tab 页 Scroller 提供，双列间距由 cardRow 控制，
 * 避免嵌套容器时边距叠加（嵌套卡片在内层卡片里也正好贴 padding）。
 */
fun ViewContainer<*, *>.groupedCard(title: String, withDivider: Boolean = true, content: ViewBuilder) {
    View {
        attr {
            flexDirectionColumn()
            backgroundColor(AdminColors.cardBg)
            borderRadius(AdminShape.radiusLg)
            border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            padding(AdminSpace.cardPadding)
            marginBottom(AdminSpace.gutter)
        }
        sectionTitle(title)
        if (withDivider) dividerH()
        content()
    }
}

// ===================== 卡片行 =====================

/**
 * 卡片行原语：一行最多两列、等宽（各 flex(1)），列间距 gutter；right 为 null 时左卡独占整行。
 * left/right 为 ViewBuilder，在对应列容器闭包内调用（接收者=列容器，节点正确挂载）。
 * 测试台各 tab 页「功能卡 + 配置卡」并排、配置页等均基于它。
 */
fun ViewContainer<*, *>.cardRow(left: ViewBuilder, right: ViewBuilder? = null) {
    View {
        attr { flexDirectionRow() }
        View {
            attr { flex(1f) }
            left()
        }
        if (right != null) {
            View { attr { width(AdminSpace.gutter) } }
            View {
                attr { flex(1f) }
                right()
            }
        }
    }
}

/**
 * 双栏页：宽屏（wide()=true）左右等宽并排，窄屏纵向堆叠。
 * wide 须为 lambda（如 `{ pagerData.pageViewWidth >= 900f }`），在 vif 闭包内读取
 * pageViewWidth（响应式字段），窗口 resize 跨过阈值时自动 并排 ⇄ 堆叠 切换。
 */
fun ViewContainer<*, *>.twoPane(wide: () -> Boolean, left: ViewBuilder, right: ViewBuilder) {
    vif({ wide() }) {
        cardRow(left, right)
    }
    velse {
        View {
            attr { flexDirectionColumn() }
            left()
            right()
        }
    }
}

// ===================== Tabs 标签页 =====================

/**
 * Tabs 标签页 UI 状态：官方 Tabs ↔ PageList 联动的两根数据线——
 * PageList `scroll` 事件回传 ScrollParams 喂给 Tabs.scrollParams，驱动指示条与 tab 选中态同步。
 * 由各状态类经 PagerScope 创建（如 form.tabUi），section 重建后当前 tab 不丢。
 */
class TabUiState(scope: PagerScope) {
    /** 当前页 index（由 PageList pageIndexDidChanged 回写），重建后 defaultPageIndex 用。 */
    var tabIndex by scope.observable(0)

    /** PageList scroll 事件回传的滚动参数（对齐官方示例：nullable，首帧未滚动时为 null）。 */
    var tabScroll by scope.observable<ScrollParams?>(null)

    /** PageList 引用：点击 tab 时 scrollToPageIndex 翻页。 */
    var pageListRef: ViewRef<PageListView<*, *>>? = null

    /** 点击 tab：滚动 PageList 到对应页（scroll 事件回流后 Tabs 指示条/选中态自动同步）。 */
    fun switchTo(index: Int) {
        pageListRef?.view?.scrollToPageIndex(index, false)
    }
}

/** 单个标签页：title = tab 项文案；content 在该页的 Scroller 闭包内执行（节点挂进该页）。 */
class TabPage(val title: String, val content: ViewBuilder)

/**
 * 标签页面板：**官方 Tabs + PageList 原生联动结构**（对齐官方文档
 * kuikly.tds.qq.com/API/components/tabs.html 与 TabsExamplePage.kt 示例，不自造样式）：
 * - tab 项 = TabItem(margin + allCenter) + Text（选中态文字色随 state.selected 切换）；
 * - 指示条 = indicatorInTabItem（absolutePosition 底部短条，官方原样）；
 * - PageList 必须显式设 pageItemWidth/pageItemHeight（官方示例同款）——不设时 item 无尺寸约束
 *   会塌成一行（实测「tab 内容都看不到」的根因）；
 * - 点击 tab → scrollToPageIndex 翻页；翻页/拖动 → scroll 事件 → ui.tabScroll → Tabs 同步。
 *
 * ⚠️ pageWidth/pageHeight 必须由调用方按自身布局算好（官方示例亦按导航/tab 高度手算），
 * 且以 lambda 传入、在 PageList attr 闭包内求值——pagerData.pageViewWidth/Height 是响应式
 * 字段，窗口 resize 时 pageItem 尺寸才会重算。
 * ⚠️ 必须在目标容器闭包内**非限定**调用（经 ctx. 调用会把节点挂到根容器，布局逃逸）。
 */
fun ViewContainer<*, *>.tabbedPanel(
    pages: List<TabPage>,
    ui: TabUiState,
    pageWidth: () -> Float,
    pageHeight: () -> Float,
) {
    Tabs {
        attr {
            indicatorAlignCenter()
            height(44f) // Tabs 源码强约束：必须显式 height，否则运行时抛错
            defaultInitIndex(ui.tabIndex)
            // 官方指示条原样：tab 项底部 absolutePosition 短条，随 scrollParams 联动滚动
            indicatorInTabItem {
                View {
                    attr {
                        absolutePosition(left = 15f, right = 15f, bottom = 5f)
                        height(6f)
                        borderRadius(3f)
                        backgroundColor(AdminColors.accent)
                    }
                }
            }
            ui.tabScroll?.also { scrollParams(it) }
        }
        pages.forEachIndexed { index, page ->
            TabItem { state ->
                attr {
                    marginLeft(10f)
                    marginRight(10f)
                    allCenter()
                }
                event {
                    click { ui.switchTo(index) }
                }
                Text {
                    attr {
                        text(page.title)
                        fontSize(AdminType.body)
                        if (state.selected) {
                            color(AdminColors.accentTintText)
                        } else {
                            color(AdminColors.textSecondary)
                        }
                    }
                }
            }
        }
    }
    PageList {
        attr {
            flexDirectionRow()
            pageItemWidth(pageWidth())
            pageItemHeight(pageHeight())
            defaultPageIndex(ui.tabIndex)
            // 首屏全量加载：官方文档要求 defaultPageIndex > 0 时 Tabs 首渲染才能正确联动到对应 item
            firstContentLoadMaxIndex(pages.size)
            offscreenPageLimit(1)
        }
        ref { ui.pageListRef = it }
        event {
            scroll { params -> ui.tabScroll = params }
            pageIndexDidChanged { params ->
                ui.tabIndex = (params as JSONObject).optInt("index")
            }
        }
        pages.forEach { page ->
            // 每个 PageList item = 一页（尺寸由 pageItemWidth/Height 约束）；页内 Scroller 纵向滚动
            View {
                attr {
                    flexDirectionColumn()
                }
                Scroller {
                    attr {
                        flex(1f)
                        paddingLeft(AdminSpace.xl)
                        paddingRight(AdminSpace.xl)
                        paddingTop(AdminSpace.xxl)
                        paddingBottom(AdminSpace.xxxl)
                    }
                    // ⚠️ ViewBuilder 值须显式传接收者调用：page.content(this) 把页面内容挂进本页 Scroller
                    // （隐式 page.content() 编译报 "No value passed for parameter 'p1'"）
                    page.content(this)
                }
            }
        }
    }
}
