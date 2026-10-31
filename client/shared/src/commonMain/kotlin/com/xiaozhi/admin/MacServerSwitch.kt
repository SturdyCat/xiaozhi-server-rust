package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.cell.Cell
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.text.input.KeyboardType
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.core.module.SharedPreferencesModule
import com.tencent.kuikly.core.nvi.serialization.json.JSONArray
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.dividerH
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.state.stateListOf
import com.xiaozhi.admin.state.stateVar

/**
 * 侧边栏「服务器设置」（macOS 专属；web 由 server 同域托管，无此概念）。
 *
 * T12 把旧 DSL 扩展 `macServerSwitchEntry` Compose 化：**状态机与发现/应用逻辑逐字保留**
 * （[ServerSwitchState]、探测、mDNS 发现、地址归一化与写回），只换渲染层——
 * mDNS 本体经 [startAdminServerDiscovery] / [stopAdminServerDiscovery] 两个平台 hook
 * 走 [XiaoZhiModule]（非 mac 平台 no-op，本入口也不渲染）。
 *
 * ## 行为口径（与旧实现一致，一条不改）
 *
 * - 启动即尝试自动连接：从未保存过地址 → 直接广播发现（开箱自动连接）；
 *   已有保存地址 → 先探一次 `/api/health`，确认旧地址连不通才允许被发现的服务器接管
 *   （用户手填的可用地址永远优先，不被自动改掉）；
 * - 保存 = 归一化写回 SP → 收起表单 → [AdminPage.refreshAll] 用新基址重跑全部数据加载；
 * - 管理令牌只存本次会话的 Pager params（`applyAdminRequestToken`），不写 SP/keychain，
 *   且切换到不同服务器时不隐式携带旧服务器的令牌（[tokenForTarget]）。
 */
internal const val KEY_SERVER_URL = "xz.server.baseUrl"
internal const val DEFAULT_SERVER = "http://127.0.0.1:8000"

internal fun adminPrefs(pager: Pager) =
    pager.acquireModule<SharedPreferencesModule>(SharedPreferencesModule.MODULE_NAME)

// 已保存地址的连通性三态：初始未知，探测失败才允许广播发现的服务器接管。
private const val SAVED_BASE_UNKNOWN = 0
private const val SAVED_BASE_USABLE = 1
private const val SAVED_BASE_UNUSABLE = 2

/** 局域网自动发现的一个服务器（服务端常驻广播 `_xiaozhi-admin._tcp`，原生桥解析回传）。 */
private data class DiscoveredServer(val name: String, val base: String)

/**
 * 表单/发现态（按 Pager 缓存，重开表单保留上次输入）。
 * ⚠️ 快照状态只允许在 Kuikly 线程写：发现回调经框架模块回调通道已切回 context 线程。
 */
private class ServerSwitchState {
    var input by stateVar(DEFAULT_SERVER)
    var busy by stateVar(false)

    // Masked connection-form draft only. Applied credential lives in this Pager's params,
    // never SP/keychain or ConfigFormState.expectedToken. Clear the draft on Apply/Cancel.
    var tokenInput by stateVar("")
    var tokenEdited = false

    /** 当前生效基址（侧边栏入口小字实时展示）。 */
    var current by stateVar("")

    /** 自动接管只发生一次：首次运行（从未保存过地址）发现到服务器即自动连接。 */
    var autoApplied = false

    /**
     * 已保存地址的连通性探测结果（[SAVED_BASE_UNKNOWN] / [SAVED_BASE_USABLE] /
     * [SAVED_BASE_UNUSABLE]）。服务器换机或换 IP/端口后旧地址会永久失效，
     * 只有确认旧地址连不通，才允许被局域网广播发现的服务器接管——
     * 用户手填的可用地址（含远程服务器）永远优先，不被自动改掉。
     */
    var savedBaseState by stateVar(SAVED_BASE_UNKNOWN)

    /** 探测只发一次（入口每次进入组合只跑一次 bootstrap）。 */
    var savedBaseProbing = false

    /** 原生 mDNS 浏览是否在跑（发现列表提示文案随它切换；防重复 start）。 */
    var discoveryOn by stateVar(false)

    /** 当前局域网内发现的服务器（每次发现变化整体替换）。 */
    val discovered: MutableList<DiscoveredServer> = stateListOf()
}

private val switchStates = mutableMapOf<Pager, ServerSwitchState>()

/**
 * 侧边栏「服务器设置」：入口项（下方小字实时显示当前地址）+ [AdminPage.serverSwitchOpen]
 * 展开的内联表单。表单 = 自动发现列表（点击即接入）+ 手填地址（保存归一化写回 SP →
 * 收起表单 → [onApplied]（壳层用新基址重跑全部数据加载））。不做浮层，避免跨端点击命中问题。
 */
@Composable
internal fun ServerSwitchEntry(page: AdminPage) {
    val pager: Pager = page
    val state = switchStates.getOrPut(pager) { ServerSwitchState() }
    if (state.current.isEmpty()) state.current = pagerBaseUrl(pager)

    // 启动即尝试自动连接（幂等）：从未配置过地址 → 直接开始广播发现（开箱自动连接）；
    // 已有保存地址 → 先探一次 /api/health，确认旧地址连不通才让发现的服务器接管。
    // 旧 DSL 在每次渲染时执行同样的守卫逻辑；Compose 里收敛为「每 Pager 一次」的效应。
    LaunchedEffect(pager) {
        if (!state.autoApplied) {
            if (adminPrefs(pager).getItem(KEY_SERVER_URL).isEmpty()) {
                startDiscovery(pager, state) { page.refreshAll(pagerBaseUrl(pager)) }
            } else {
                probeSavedBase(pager, state) { page.refreshAll(pagerBaseUrl(pager)) }
            }
        }
    }

    Column(modifier = Modifier.fillMaxWidth()) {
        Cell(
            title = "服务器设置",
            onClick = {
                state.input = adminPrefs(pager).getItem(KEY_SERVER_URL).ifEmpty { DEFAULT_SERVER }
                state.tokenInput = pager.adminRequestToken()
                state.tokenEdited = false
                state.busy = false
                page.serverSwitchOpen = true
                // 面板展开期间保持发现（列表实时刷新）；已在跑则复用（幂等）。
                startDiscovery(pager, state) { page.refreshAll(pagerBaseUrl(pager)) }
            },
            // 与导航项同一套选中信号：品牌绿文字（选中竖条留给真导航项，这里不用）
            titleColor = if (page.serverSwitchOpen) Theme.colors.primary else null,
        )
        Text(
            text = "当前：${state.current}",
            modifier = Modifier
                .fillMaxWidth()
                .padding(start = Spacing.lg, end = Spacing.lg, bottom = LocalAdminSpacing.current.xs),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        // 旧地址连不通时明确说出来：服务器换机/改端口后用户只看「连不上」是猜不到原因的。
        if (state.savedBaseState == SAVED_BASE_UNUSABLE && state.discovered.isEmpty()) {
            Text(
                text = "当前地址连不通，正在搜索局域网内的服务器…",
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(start = Spacing.lg, end = Spacing.lg, bottom = LocalAdminSpacing.current.xs),
                style = Theme.typography.caption,
                color = Theme.colors.destructive,
            )
        }
        if (page.serverSwitchOpen) {
            ServerSwitchPanel(page, pager, state)
        }
    }
}

/** 展开的内联表单：发现列表 + 手填地址 + 会话令牌。容器是 240 宽的侧边栏，纵向堆叠。 */
@Composable
private fun ServerSwitchPanel(page: AdminPage, pager: Pager, state: ServerSwitchState) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = LocalAdminSpacing.current.sm, end = LocalAdminSpacing.current.sm, bottom = LocalAdminSpacing.current.sm),
    ) {
        Text(
            text = "局域网自动发现",
            modifier = Modifier.padding(top = LocalAdminSpacing.current.xs),
            style = Theme.typography.label,
            color = Theme.colors.foreground,
        )
        Text(
            text =
                if (state.discovered.isEmpty()) {
                    if (state.discoveryOn) "正在搜索同网段的小智服务端…" else "未发现（服务端需已启动）"
                } else {
                    "点击即连接"
                },
            modifier = Modifier.padding(top = 2.dp),
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
        )
        for (server in state.discovered) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable {
                        applyBase(
                            pager, state, server.base,
                            onClosed = { page.serverSwitchOpen = false },
                            onApplied = { page.refreshAll(pagerBaseUrl(pager)) },
                            requestToken = tokenForTarget(state, server.base),
                        )
                    }
                    .padding(top = LocalAdminSpacing.current.xs, bottom = LocalAdminSpacing.current.xs),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = server.name,
                    modifier = Modifier.weight(1f),
                    style = Theme.typography.caption,
                    color = Theme.colors.foreground,
                )
                Text(
                    text = server.base,
                    style = Theme.typography.caption,
                    color = Theme.colors.mutedForeground,
                )
            }
        }
        dividerH()
        labeledField(
            label = "服务器地址",
            value = state.input,
            onValueChange = {
                state.input = it
                // 换目标地址后旧令牌草稿不能隐式跟随（不归属于同一权威）
                if (normalizeBase(it) != state.current && !state.tokenEdited) state.tokenInput = ""
            },
            placeholder = DEFAULT_SERVER,
        )
        // 会话令牌：password 键盘；原生 textInputState 通道保持 IME 安全（无受控回写循环）
        labeledField(
            label = "管理令牌（仅本次会话）",
            value = state.tokenInput,
            onValueChange = {
                state.tokenInput = it
                state.tokenEdited = true
            },
            placeholder = "留空不携带管理令牌",
            keyboardType = KeyboardType.Password,
            note = "仅用于当前管理请求，不写入服务端配置或本地存储。切换服务器请重新输入令牌。",
        )
        actionRow {
            Button(
                text = if (state.busy) "加载中…" else "保存并重新加载",
                onClick = {
                    val base = normalizeBase(state.input)
                    if (base == null) {
                        state.input = "无效地址"
                        return@Button
                    }
                    state.busy = true
                    // 手动选择是最高优先级：写回 SP 后自动接管永久退场（尊重用户选择）。
                    state.autoApplied = true
                    stopDiscovery(pager, state)
                    applyBase(
                        pager, state, base,
                        onClosed = { page.serverSwitchOpen = false },
                        onApplied = { page.refreshAll(pagerBaseUrl(pager)) },
                        requestToken = tokenForTarget(state, base),
                    )
                },
                type = ButtonType.FILL,
                loading = state.busy,
                disabled = state.busy,
            )
            Button(
                text = "取消",
                onClick = {
                    state.tokenInput = ""
                    state.tokenEdited = false
                    page.serverSwitchOpen = false
                    // 自动接管不再可能（有手填记忆）或用户主动收起且已接管 → 停止常驻浏览。
                    if (!autoDiscoveryNeeded(pager, state)) stopDiscovery(pager, state)
                },
                type = ButtonType.OUTLINE,
            )
        }
    }
}

/** Never forward a previous server's credential implicitly to another authority. */
private fun tokenForTarget(state: ServerSwitchState, base: String): String =
    if (base == state.current || state.tokenEdited) state.tokenInput else ""

// ------------------------------------------------------------
// 发现与地址应用（自旧 macosArm64Main AdminPlatform.kt 逐字搬来，仅 prefs 改名 adminPrefs）
// ------------------------------------------------------------

/** 是否还需要常驻发现：仅在「未自动接管且（从未保存地址 或 保存的地址已连不通）」时为真。 */
private fun autoDiscoveryNeeded(pager: Pager, state: ServerSwitchState): Boolean =
    !state.autoApplied &&
        (adminPrefs(pager).getItem(KEY_SERVER_URL).isEmpty() || state.savedBaseState == SAVED_BASE_UNUSABLE)

/**
 * 探测已保存地址是否还活着（GET /api/health）。连不通 = 服务器换机/换 IP/改端口，
 * 此时才启用广播发现接管，让客户端自己恢复，不用用户去猜新地址。
 */
private fun probeSavedBase(pager: Pager, state: ServerSwitchState, onApplied: () -> Unit) {
    if (state.savedBaseProbing) return
    state.savedBaseProbing = true
    val base = adminPrefs(pager).getItem(KEY_SERVER_URL)
    network(pager).httpRequest("$base/api/health", false, JSONObject(), JSONObject(), timeout = 5) { _, success, _, resp ->
        // 只有明确的 HTTP 2xx 才算可用；超时/连接拒绝/5xx 一律视为失效。
        val usable = success && (resp.statusCode == null || resp.statusCode in 200..299)
        state.savedBaseState = if (usable) SAVED_BASE_USABLE else SAVED_BASE_UNUSABLE
        if (!usable && !state.autoApplied) {
            // 旧地址已死：启动广播发现，命中即接管（applyBase 写回新地址并重载全部数据）。
            startDiscovery(pager, state, onApplied)
        }
    }
}

/** 启动 mDNS 发现（幂等）；回调（已在 Kuikly 线程）整体替换列表并尝试自动接管。 */
private fun startDiscovery(pager: Pager, state: ServerSwitchState, onApplied: () -> Unit) {
    if (state.discoveryOn) return
    state.discoveryOn = true
    pager.startAdminServerDiscovery { result ->
        val servers = parseServers(result)
        state.discovered.clear()
        state.discovered.addAll(servers)
        if (autoDiscoveryNeeded(pager, state)) {
            servers.firstOrNull()?.let { first ->
                state.autoApplied = true
                stopDiscovery(pager, state)
                // Automatic discovery may select an address, never an uncommitted token draft.
                applyBase(pager, state, first.base, onClosed = {}, onApplied = onApplied,
                    requestToken = if (first.base == state.current) pager.adminRequestToken() else "")
            }
        }
    }
}

private fun stopDiscovery(pager: Pager, state: ServerSwitchState) {
    if (!state.discoveryOn) return
    state.discoveryOn = false
    pager.stopAdminServerDiscovery()
}

/** 解析原生回包 {servers:[{name, base}]}；结构异常返回已解析部分（不抛）。 */
private fun parseServers(result: JSONObject?): List<DiscoveredServer> {
    val arr: JSONArray = result?.optJSONArray("servers") ?: return emptyList()
    val out = mutableListOf<DiscoveredServer>()
    for (i in 0 until arr.length()) {
        val o = arr.optJSONObject(i) ?: continue
        val base = o.optString("base", "")
        if (base.isNotEmpty()) out.add(DiscoveredServer(o.optString("name", "小智服务端"), base))
    }
    return out
}

/** 写回地址 → 收起表单 → 通知壳层用新基址重载全部数据（自动接管与手动保存共用）。 */
private fun applyBase(
    pager: Pager,
    state: ServerSwitchState,
    base: String,
    onClosed: () -> Unit,
    onApplied: () -> Unit,
    requestToken: String,
) {
    adminPrefs(pager).setItem(KEY_SERVER_URL, base)
    pager.applyAdminRequestToken(requestToken)
    state.current = base
    state.input = base
    state.tokenInput = ""
    state.tokenEdited = false
    state.busy = false
    onClosed()
    onApplied()
}

/** 地址归一化（收编自原 ConnectState）：容忍无 scheme / ws(s):// 前缀 / 误带路径，只保留 authority。 */
private fun normalizeBase(raw: String): String? {
    var s = raw.trim()
    if (s.isEmpty()) return null
    s = when {
        s.startsWith("ws://", true) -> "http://" + s.substring(5)
        s.startsWith("wss://", true) -> "https://" + s.substring(6)
        else -> s
    }
    if (!s.contains("://")) s = "http://$s"
    val schemeEnd = s.indexOf("://")
    val scheme = s.substring(0, schemeEnd).lowercase()
    if (scheme != "http" && scheme != "https") return null
    val authority = s.substring(schemeEnd + 3).split('/', '?', '#').firstOrNull() ?: ""
    if (authority.isEmpty() || authority.startsWith(":")) return null
    return "$scheme://$authority"
}
