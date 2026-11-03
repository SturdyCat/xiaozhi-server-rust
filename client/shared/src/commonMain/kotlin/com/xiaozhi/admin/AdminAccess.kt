package com.xiaozhi.admin


import com.tencent.kuikly.core.pager.Pager

private const val ADMIN_ACCESS_REVISION = "_xz_admin_access_revision"

/** Actual request credential: Web launch params or the macOS session-only connection form. */
fun Pager.adminRequestToken(): String = pagerData.params.optString("token", "")

// 逐请求头（adminRequestHeaders）已随「管理台 API 全走 WS RPC」移除：鉴权在 WS 升级时
// 完成（token 编码进 ?token= 查询参数），JSON 请求体由 rpc 帧层自带 Content-Type。

/** Non-secret epoch rejects late replies even after an A→B→A credential switch. */
internal fun Pager.adminAccessRevision(): Long = pagerData.params.optLong(ADMIN_ACCESS_REVISION, 0L)

internal fun Pager.isAdminAccessCurrent(revision: Long, token: String): Boolean =
    revision == adminAccessRevision() && token == adminRequestToken()

/**
 * Context-thread only, explicit connection-form Apply. Params are mutable for this Pager's
 * lifetime; this writes neither SP/keychain nor configuration drafts, request bodies or logs.
 * Existing onApplied→form.load also invalidates the owning read-only request gates.
 */
internal fun Pager.applyAdminRequestToken(token: String) {
    val revision = adminAccessRevision() + 1L
    pagerData.params.put("token", token)
    pagerData.params.put(ADMIN_ACCESS_REVISION, revision)
}
