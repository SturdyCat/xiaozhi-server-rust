package com.xiaozhi.admin

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vfor
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.ScrollParams
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.TabItem
import com.tencent.kuikly.core.views.Tabs
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 配置页主体——**三级导航布局**：
 *
 * ```
 * 一级  管理台侧边栏（纵向，AdminNav：概览/设备/…/配置）——本函数不负责
 * 二级  配置内容区顶部的横向 tabs（官方 Tabs：总览 | 预设 | 服务与接入 | 语音与对话 | 智能体）
 * 三级  分组 tab 内的纵向子菜单（左列）+ 当前 section 内容（右栏）
 * ```
 *
 * 状态单源：只有 `form.configSection` 一个选中态，二级 tab 由它**派生**
 * （[ConfigSections.tabOf]），点 tab = 选中该 tab 的第一个 section
 * （[ConfigSections.firstSectionIn]）。因此外部跳转（总览横幅/设备个性化入口改
 * `configSection`）后本布局随 observable 自动重渲染，不会出现 tab 与内容错位。
 *
 * 搜索（`form.configSearch` 非空）时临时替换为平铺命中项列表，点击命中项选中并清空搜索。
 *
 * ⚠️ Kuikly DSL 约束：渲染扩展必须在目标容器闭包内**非限定**调用（经 `ctx.` 调用会把
 * 节点挂到根容器导致布局逃逸）；vfor 项闭包内必须且仅一个孩子节点（2.29.1 严格校验）。
 */
fun ViewContainer<*, *>.renderConfigLayout(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
    ctx: Pager,
) {
    View {
        attr {
            flex(1f)
            flexDirectionColumn()
            backgroundColor(AdminColors.windowBg)
        }

        // ===== 搜索态：平铺命中项（临时替换 tabs + 内容，点击后回到三级导航）=====
        vif({ form.configSearch.isNotBlank() }) {
            configSearchResultPane(form, meta, ctx)
        }

        // ===== 三级导航态 =====
        vif({ form.configSearch.isBlank() }) {
            // —— 二级：横向 tabs 行（官方 Tabs 指示条）+ 右侧搜索框 ——
            View {
                attr {
                    flexDirectionRow()
                    alignItemsCenter()
                    paddingRight(AdminSpace.md)
                }
                configTabBar(form)
                View { attr { flex(1f) } }
                View {
                    attr { width(220f) }
                    configSearchField(form)
                }
            }
            View {
                attr { height(1f); backgroundColor(AdminColors.divider) }
            }

            // —— 三级：tab 内容。置顶项 tab 直接渲染该 section；分组 tab = 子菜单 + 内容 ——
            for (d in ConfigSections.pinned) {
                vif({ form.configSection == d.id }) {
                    configContentPane(form, meta, health, mcp, ota, toast, ctx)
                }
            }
            for ((gid, _) in ConfigSections.groups) {
                vif({ ConfigSections.tabOf(form.configSection) == gid }) {
                    configGroupPane(gid, form, meta, health, mcp, ota, toast, ctx)
                }
            }
        }
    }
}

/**
 * tab 项等宽节距：官方 Tabs 指示条位置由 scrollParams 驱动（见 TabsView 源码，
 * defaultInitIndex 只在首建生效、TabItem 点击不直接推动指示条），本页无配对 PageList，
 * 由 configSection 派生 offset 手动喂 ScrollParams（等宽节距使 offset/pitch 恰为选中 index）。
 */
private const val TAB_ITEM_WIDTH = 96f

/** 二级横向 tab 条（官方 Tabs + TabItem + 底部指示条，样式对齐 AdminComponents.tabbedPanel）。
 *  选中态单源 = `form.configSection`：点击与外部跳转都改它，attr 内派生 index 喂 scrollParams，
 *  指示条与 TabItem 高亮随 observable 重渲染自动跟随。
 */
private fun ViewContainer<*, *>.configTabBar(form: ConfigFormState) {
    val pitch = TAB_ITEM_WIDTH + 2 * AdminSpace.md
    val tabCount = ConfigSections.tabs.size
    Tabs {
        attr {
            flex(1f)
            indicatorAlignCenter()
            height(44f) // Tabs 源码强约束：必须显式 height，否则运行时抛错
            defaultInitIndex(
                ConfigSections.tabs.indexOfFirst { it.first == ConfigSections.tabOf(form.configSection) }
                    .coerceAtLeast(0),
            )
            // 响应式读取 configSection：变化时重建 ScrollParams → 指示条动画滑动到目标项
            val selectedIndex =
                ConfigSections.tabs.indexOfFirst { it.first == ConfigSections.tabOf(form.configSection) }
                    .coerceAtLeast(0)
            scrollParams(
                ScrollParams(
                    offsetX = selectedIndex * pitch,
                    offsetY = 0f,
                    contentWidth = pitch * tabCount,
                    contentHeight = 44f,
                    viewWidth = pitch,
                    viewHeight = 44f,
                    isDragging = false,
                ),
            )
            indicatorInTabItem {
                View {
                    attr {
                        absolutePosition(left = 15f, right = 15f, bottom = 5f)
                        height(3f)
                        borderRadius(AdminShape.radiusPill)
                        backgroundColor(AdminColors.accent)
                    }
                }
            }
        }
        for ((tabId, tabLabel) in ConfigSections.tabs) {
            TabItem { state ->
                attr {
                    width(TAB_ITEM_WIDTH)
                    marginLeft(AdminSpace.md)
                    marginRight(AdminSpace.md)
                    allCenter()
                }
                event {
                    click { form.configSection = ConfigSections.firstSectionIn(tabId) }
                }
                Text {
                    attr {
                        text(tabLabel)
                        fontSize(AdminType.body)
                        color(
                            if (state.selected) AdminColors.accentTintText else AdminColors.textSecondary,
                        )
                    }
                }
            }
        }
    }
}

/** 分组 tab 的三级层：左列纵向子菜单（组内 section）+ 右栏当前 section 内容。 */
private fun ViewContainer<*, *>.configGroupPane(
    groupId: String,
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
    ctx: Pager,
) {
    View {
        attr {
            flex(1f)
            flexDirectionRow()
        }
        // 左列：纵向子菜单
        View {
            attr {
                width(200f)
                flexDirectionColumn()
                backgroundColor(AdminColors.sidebarBg)
                paddingTop(AdminSpace.sm)
                paddingBottom(AdminSpace.lg)
            }
            configGroupHeader(ConfigSections.groups.first { it.first == groupId }.second)
            Scroller {
                attr { flex(1f) }
                for (d in ConfigSections.sectionsIn(groupId)) {
                    sidebarItem(
                        d.label,
                        isSelected = { form.configSection == d.id },
                        showDot = { sectionCapability(meta, d.id)?.phase in setOf("failed", "degraded") },
                        onClick = { form.configSection = d.id },
                    )
                }
            }
        }
        // 子菜单右侧 1px 分隔
        View {
            attr { width(1f); backgroundColor(AdminColors.divider) }
        }
        // 右栏：当前 section 内容
        configContentPane(form, meta, health, mcp, ota, toast, ctx)
    }
}

/** section 内容栏：`vfor(configSectionList)` 单元素技巧驱动响应式重建（选中变化必重渲染）。 */
private fun ViewContainer<*, *>.configContentPane(
    form: ConfigFormState,
    meta: PluginMetaState,
    health: EngineHealthState,
    mcp: McpToolsState,
    ota: OtaState,
    toast: ToastState,
    ctx: Pager,
) {
    View {
        attr {
            flex(1f)
            flexDirectionColumn()
        }
        opsScroller {
            vfor({ form.configSectionList }) { sectionId ->
                renderConfigSection(form, meta, health, mcp, ota, toast, ctx, sectionId)
            }
        }
    }
}

/** 搜索态面板：平铺命中项列表（复用 sidebarItem 样式），点击选中并退出搜索。 */
private fun ViewContainer<*, *>.configSearchResultPane(
    form: ConfigFormState,
    meta: PluginMetaState,
    ctx: Pager,
) {
    opsScroller {
        val hits = ConfigSections.search(form.configSearch)
        if (hits.isEmpty()) {
            textNote("没有匹配「${form.configSearch}」的配置项", micro = true)
        } else {
            for (d in hits) {
                sidebarItem(
                    d.label,
                    isSelected = { form.configSection == d.id },
                    showDot = { sectionCapability(meta, d.id)?.phase in setOf("failed", "degraded") },
                    onClick = {
                        form.configSection = d.id
                        form.configSearch = ""
                    },
                )
            }
        }
    }
}
