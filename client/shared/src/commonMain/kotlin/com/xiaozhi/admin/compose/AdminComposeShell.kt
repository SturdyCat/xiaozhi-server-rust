package com.xiaozhi.admin.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.tencent.kuikly.compose.ComposeContainer
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxHeight
import com.tencent.kuikly.compose.foundation.layout.fillMaxSize
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.height
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.layout.width
import com.tencent.kuikly.compose.foundation.lazy.LazyColumn
import com.tencent.kuikly.compose.material3.Text
import com.tencent.kuikly.compose.setContent
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.text.font.FontWeight
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.compose.ui.unit.sp
import com.tencent.kuikly.core.annotations.Page
import com.tencent.kuikly.core.module.RouterModule
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.IS_MACOS
import com.xiaozhi.admin.ui.AdminNav
import com.xiaozhi.admin.ui.AdminSpace
import com.xiaozhi.admin.ui.AdminType

/**
 * 管理台 · Compose 壳（`page_name=admin-compose`）——全量迁移的**目标架构**。
 *
 * ## 与旧壳的关系
 * 旧管理台是 `@Page("admin")` 的纯 DSL 单页（侧栏 + 标题栏 + 11 个 section 全在
 * `View { }` 里）。Compose 无法嵌进 DSL 布局，所以迁移只能"整页换壳"：本页是替换目标，
 * 逐段把 section 搬进来，搬完一段就把旧壳里的入口切过来，全部搬完后删除旧壳。
 *
 * ## 导航不变
 * 侧栏项直接复用 [AdminNav] 常量表（id/order/label/macOnly 与旧壳同源），
 * 不另建第二份导航数据——否则两个壳的菜单顺序会漂移。
 *
 * ## 本节范围
 * 当前已完成：「全局配置」（见 [AdminSettingsSection]）。
 * 其余 section 渲染迁移占位并提供「回到旧版」出口，保证任一菜单都不会点进空白页。
 */
@Page("admin-compose")
internal class AdminComposeShellPage : ComposeContainer() {

    override fun willInit() {
        super.willInit()
        // 字号随视口缩放（与旧壳 AdminPage.body() 同一口径，1.0~1.5）
        AdminType.scale = AdminType.scaleFor(pagerData.pageViewWidth)
        val page = this
        setContent {
            AdminComposeTheme {
                AdminComposeShell(
                    pager = page,
                    pageHeight = page.pagerData.pageViewHeight,
                    onOpenLegacy = { section ->
                        page.acquireModule<RouterModule>(RouterModule.MODULE_NAME)
                            .openPage("admin", com.tencent.kuikly.core.nvi.serialization.json.JSONObject().apply {
                                put("initial_section", section)
                            })
                    },
                )
            }
        }
    }
}

/** 侧栏宽度与旧壳保持一致（[com.xiaozhi.admin.ADMIN_SIDEBAR_WIDTH]）。 */
private val SidebarWidth = 240.dp
private val SidebarItemHeight = 44.dp
private val GroupLabelTopPadding = 16.dp
private val ShellTopInset = 36.dp

@Composable
private fun AdminComposeShell(pager: Pager, pageHeight: Float, onOpenLegacy: (String) -> Unit) {
    var selected by remember {
        mutableStateOf(
            pager.pageData.params.optString("nav", "").let { s ->
                if (s.isNotEmpty() && AdminNav.visible(IS_MACOS).any { it.id == s }) s else "settings"
            },
        )
    }

    // ⚠️ 高度用显式 pageViewHeight：Kuikly Compose 的根约束下 `fillMaxHeight` 会退化成
    // wrap content（侧栏只包住导航项、不到窗口底），实测于 admin-compose 首版。
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(pageHeight.dp)
            .background(AdminC.windowBg),
    ) {
        AdminSidebar(selected = selected, pageHeight = pageHeight, onSelect = { selected = it })
        // 侧栏 │ 内容：1dp 竖线用独立 Box 表达（Compose 的 border 只能四边同色，这里只要左边）
        Box(modifier = Modifier.width(1.dp).fillMaxHeight().background(AdminC.divider))
        Box(modifier = Modifier.weight(1f).height(pageHeight.dp)) {
            Column(modifier = Modifier.fillMaxSize()) {
                AdminTitleBar(title = AdminNav.label(selected))
                when (selected) {
                    "settings" -> AdminSettingsSection(pager = pager, onOpenLegacy = onOpenLegacy)
                    else -> MigrationPlaceholder(
                        sectionLabel = AdminNav.label(selected),
                        onOpenLegacy = { onOpenLegacy(selected) },
                    )
                }
            }
        }
    }
}

@Composable
private fun AdminSidebar(selected: String, pageHeight: Float, onSelect: (String) -> Unit) {
    Column(
        modifier = Modifier
            .width(SidebarWidth)
            .height(pageHeight.dp)
            .background(AdminC.sidebarBg)
            .padding(top = ShellTopInset),
    ) {
        LazyColumn(modifier = Modifier.weight(1f)) {
            item {
                var lastGroup = ""
                for (nav in AdminNav.visible(IS_MACOS)) {
                    if (nav.group != lastGroup) {
                        lastGroup = nav.group
                        Text(
                            text = nav.group,
                            color = AdminC.textTertiary,
                            fontSize = AdminType.micro.sp,
                            modifier = Modifier.padding(
                                start = (AdminSpace.md + AdminSpace.sm).dp,
                                top = GroupLabelTopPadding,
                                bottom = 4.dp,
                            ),
                        )
                    }
                    SidebarItem(
                        label = nav.label,
                        selected = selected == nav.id,
                        onClick = { onSelect(nav.id) },
                    )
                }
            }
        }
        Text(
            text = "小智服务端 · 管理台",
            color = AdminC.textTertiary,
            fontSize = AdminType.micro.sp,
            modifier = Modifier.padding(
                start = (AdminSpace.md + AdminSpace.sm).dp,
                bottom = AdminSpace.md.dp,
            ),
        )
    }
}

@Composable
private fun SidebarItem(label: String, selected: Boolean, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(SidebarItemHeight)
            .background(if (selected) AdminC.sidebarSelectedBg else AdminC.transparent)
            .clickable { onClick() },
    ) {
        // 选中态左侧 3dp 品牌绿竖条（与 DSL 侧 menuListItem MAIN 档同一视觉）
        if (selected) {
            Box(
                modifier = Modifier
                    .width(3.dp)
                    .fillMaxHeight()
                    .background(AdminC.accent),
            )
        }
        Text(
            text = label,
            color = if (selected) AdminC.textAccent else AdminC.textPrimary,
            fontSize = AdminType.body.sp,
            modifier = Modifier.align(Alignment.CenterStart).padding(start = AdminSpace.sm.dp),
        )
    }
}

@Composable
private fun AdminTitleBar(title: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(56.dp)
            .background(AdminC.windowBg)
            .padding(horizontal = AdminSpace.pagePadding.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // 与旧壳 largeTitleBar 的 PRIMARY 档一致：title(24) + Bold
        Text(
            text = title,
            color = AdminC.textPrimary,
            fontSize = AdminType.title.sp,
            fontWeight = FontWeight.Bold,
        )
    }
}

/** 未迁移 section 的占位：明确告知状态并给旧版入口，不做半成品表单。 */
@Composable
private fun MigrationPlaceholder(sectionLabel: String, onOpenLegacy: () -> Unit) {
    Column(modifier = Modifier.fillMaxSize().padding(AdminSpace.pagePadding.dp)) {
        Text(
            text = "「$sectionLabel」正在迁移到 Compose 磁贴",
            color = AdminC.textPrimary,
            fontSize = AdminType.body.sp,
        )
        Text(
            text = "该页暂未迁移，功能仍在旧版管理台完整可用。",
            color = AdminC.textSecondary,
            fontSize = AdminType.caption.sp,
            modifier = Modifier.padding(top = AdminSpace.xs.dp),
        )
        Text(
            text = "前往旧版 →",
            color = AdminC.textAccent,
            fontSize = AdminType.body.sp,
            modifier = Modifier.padding(top = AdminSpace.sm.dp).clickable { onOpenLegacy() },
        )
    }
}

/** 便捷取 Pager（Compose 内需要发请求时用）。 */
internal fun Pager.router(): RouterModule = acquireModule(RouterModule.MODULE_NAME)
