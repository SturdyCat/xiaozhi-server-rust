package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.pager.Pager

private const val ADMIN_ACCESS_REVISION = "_xz_admin_access_revision"

/** Actual request credential: Web launch params or the macOS session-only connection form. */
fun Pager.adminRequestToken(): String = pagerData.params.optString("token", "")

/** Build a fresh header object per request. Never use ConfigFormState.expectedToken here. */
fun adminRequestHeaders(token: String): JSONObject = JSONObject().apply {
    put("Content-Type", "application/json")
    if (token.isNotEmpty()) put("Authorization", "Bearer $token")
}

fun Pager.adminRequestHeaders(): JSONObject = adminRequestHeaders(adminRequestToken())

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
