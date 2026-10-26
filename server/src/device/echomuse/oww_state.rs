//! OWW 资产现状（只读五态）：`GET /api/echomuse/devices/{id}/oww/state`。
//!
//! 为什么单独成文件而不塞进 [`super::oww`]：那个文件负责「下载 → 转换 → 下发」整条链路
//! （已 700+ 行），本文件只做**读**：一趟受限 shell 拿设备清单 → 与服务端待下发资产比对
//! → 五态回包。判定全部是纯函数，供 `oww_state_tests.rs` 直驱（不连设备、不下发）。
//!
//! 最关键的一条语义：**读不到设备 ≠ 缺失**（官方 `em_oww_sync` 的 "failure to LOOK is
//! not evidence of absence"）。shell 面可能还没起来（设备刚接入），此时报 `missing` 会
//! 让管理台催着"重装"，而真实情况只是"暂时读不到"——所以一律 `unknown`。

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::json;

use super::oww::{
    device_state, ort_asset, sha256_hex, vad_asset, DeviceOwwState, FREE_FLOOR_MB, ORT_SO_FILE,
    VAD_NAME,
};
use super::{admin, registry};
use crate::runtime::Engines;

/// 挂载路由（由 [`super::oww::routes`] merge，鉴权/闸门与 OWW 其它接口同源）。
pub(super) fn routes() -> axum::Router<Arc<Engines>> {
    use axum::routing::get;
    axum::Router::new().route("/api/echomuse/devices/{id}/oww/state", get(get_state))
}

/// 单件资产的五种状态。字面量与前端 `EchoMuseUpdatesTab` 逐字一致（设计 §3.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AssetState {
    /// 设备上那份摘要与服务端要下发的那份一致。
    Installed,
    /// 设备上有一份，但摘要与服务端的不同（需要重下发）。
    Outdated,
    /// 设备上缺这件，且剩余空间低于 [`FREE_FLOOR_MB`]——现在推也大概率失败。
    Blocked,
    /// 判定不出来：读不到设备，或服务端自己没有可比对的资产。
    Unknown,
    /// 设备上确实没有这件（清单读到了，就是没这个文件）。
    Missing,
}

impl AssetState {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Outdated => "outdated",
            Self::Blocked => "blocked",
            Self::Unknown => "unknown",
            Self::Missing => "missing",
        }
    }
}

/// 五态的闭合取值集合（回包 `states` 字段自描述，前端据此校验而不是猜）。
pub(super) const ASSET_STATES: [&str; 5] =
    ["installed", "outdated", "blocked", "unknown", "missing"];

/// 单件资产状态判定（纯函数）。
///
/// - `device == None`（清单读不到）→ [`AssetState::Unknown`]，**永不**是 `Missing`；
/// - 设备有文件且与服务端摘要一致（大小写不敏感）→ `Installed`；
/// - 设备有文件但服务端没资产可比（未就绪/不可解析）→ `Unknown`（新鲜度无从判定）；
/// - 设备有文件但摘要不同 → `Outdated`；
/// - 设备没这文件 → 空间低于下限 `Blocked`，否则 `Missing`。
pub(super) fn asset_state(
    device: Option<&HashMap<String, String>>,
    name: &str,
    expected: Option<&str>,
    free_mb: Option<u64>,
) -> AssetState {
    let Some(files) = device else {
        return AssetState::Unknown;
    };
    match files.get(name) {
        Some(got) => match expected {
            Some(want) if want.eq_ignore_ascii_case(got) => AssetState::Installed,
            None => AssetState::Unknown,
            Some(_) => AssetState::Outdated,
        },
        None => {
            if free_mb.is_some_and(|free| free < FREE_FLOOR_MB) {
                AssetState::Blocked
            } else {
                AssetState::Missing
            }
        }
    }
}

/// `/oww/state` 回包体（纯构造，测试直驱）。
///
/// `vad_expected`/`ort_expected` 是**服务端将要下发**的摘要（`None` = 服务端资产未就绪）；
/// `device` 是设备侧清单（`None` = 没读到）。缺失一律 `null` / `unknown`，绝不假填 `0`
/// 或 `installed`。
pub(super) fn oww_state_payload(
    id: &str,
    device: Option<&DeviceOwwState>,
    vad_expected: Option<(&str, usize)>,
    ort_expected: Option<&str>,
    checked_at: u64,
) -> serde_json::Value {
    let files = device.map(|state| &state.files);
    let free_mb = device.and_then(|state| state.free_mb);
    let vad = asset_state(
        files,
        VAD_NAME,
        vad_expected.map(|(digest, _)| digest),
        free_mb,
    );
    let ort = asset_state(files, ORT_SO_FILE, ort_expected, free_mb);
    json!({
        "device_id": id,
        "vad": {
            "state": vad.as_str(),
            "file": VAD_NAME,
            "sha256": vad_expected.map(|(digest, _)| digest),
            "bytes": vad_expected.map(|(_, len)| len),
        },
        "ort": {
            "state": ort.as_str(),
            "file": ORT_SO_FILE,
            "sha256": ort_expected,
        },
        "free_mb": free_mb,
        // 自描述取值集合：前端据此校验，不硬编码想象出来的第六种状态。
        "states": ASSET_STATES,
        "checked_at": checked_at,
    })
}

/// `GET /api/echomuse/devices/{id}/oww/state`：设备侧 OWW 资产现状（只读）。
///
/// 鉴权与系统接口同源（[`super::system::system_gate`]：管理面鉴权 + 已批准 emOS +
/// 在线且已认证 control + peer 存在）。**读不到设备不是错误**：返回 200 + `unknown`
/// （与自动同步 `oww::ensure_on_device_assets` 同一口径）。
async fn get_state(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Err(response) = super::system::system_gate(&engines, &headers, &query, &id) {
        return response;
    }
    // 服务端视角的"应该是什么"：读不出来只意味着无法比对，不影响本接口可用性。
    let vad = vad_asset()
        .ok()
        .map(|bytes| (sha256_hex(&bytes), bytes.len()));
    let ort = ort_asset().ok().map(|bytes| sha256_hex(&bytes));
    let device = device_state(&id).await;
    let body = oww_state_payload(
        &id,
        device.as_ref(),
        vad.as_ref().map(|(digest, len)| (digest.as_str(), *len)),
        ort.as_deref(),
        registry::now_secs(),
    );
    admin::json(StatusCode::OK, body)
}

#[cfg(test)]
#[path = "oww_state_tests.rs"]
mod tests;
