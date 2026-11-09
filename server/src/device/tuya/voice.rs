//! NLU iot 域语音控制（`tuya.control` 能力，首期：智能插座开关）。
//!
//! ## 职责与边界（架构 §1 D2）
//!
//! skills 目录只做能力声明与校验（不联网）；真正的名称匹配、开关 DP 识别与
//! 控制下发都在这里（device/tuya 域知识），内部复用既有 [`TuyaEngine`] 的
//! `list_devices` / `cached_dps` / `set_dp`，不新建网络通道。
//!
//! - [`TuyaEngine::voice_control`]：**infallible**，恒返回 `{"ok":bool,"speech":str}`
//!   （对齐 weather.forecast「业务失败也是 Ok」约定）——所有失败路径都组装成
//!   语音话术收尾，会话永不中断（PRD P0-9）。
//! - 纯函数（可脱离网络单测，见 `voice_tests`）：[`match_devices`]（名称匹配）、
//!   [`find_switch_dp`]（开关 DP 识别）、话术组装函数族。
//!
//! ## 名称匹配算法（架构 §2）
//!
//! 规范化（trim + 去空白与「的」）后：query 长度 <2 直接未找到（防「的」等碎词
//! 误命中）；① 精确 alias == query（优先）→ name == query；② 双向包含
//! （query ⊆ 名 或 名 ⊆ query，「客厅」与「客厅的插座」都要命中「客厅插座」）；
//! 命中 1 台执行、≥2 台歧义（不自动选择）、0 台未找到。离线设备参与匹配：
//! 命中离线时话术提示「离线」而非「未找到」，让用户知道设备存在但不可控。
//!
//! ## 开关 DP 识别（架构 §3）
//!
//! `cached_dps` 有值：取值为 bool 的 DP，`id=="1"` 优先，无 "1" 取任一 bool DP；
//! 缓存为 None：先做一次 `status()` 回读填充（60s 轮询间隙/重启后的首条指令
//! 不应误报「不支持开关」），回读失败自然落入离线/超时话术，仍无 bool DP 归
//! unsupported。

use std::collections::HashMap;

use serde_json::{json, Value};

use super::lan::{self, ControlFailure};
use super::store::DeviceRow;
use super::TuyaEngine;

/// 名称匹配三分类（架构 §2 ③）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NameMatch {
    /// 唯一命中：执行控制。
    Unique(DeviceRow),
    /// 多台命中：歧义话术（最多列 3 个标签，不自动选择）。
    Ambiguous(Vec<DeviceRow>),
    /// 无命中（含 query 过短）。
    NotFound,
}

/// 歧义话术最多列出的候选数（语音场景再长用户记不住）。
const AMBIGUOUS_LIMIT: usize = 3;

/// 语音设备名规范化：trim + 去空白与「的」。
///
/// 「的」是中文口语最高频的衬字（「客厅的插座」），去掉后才能与目录名
/// 「客厅插座」精确/包含命中；设备名里含「的」属罕见命名，两侧都做同一
/// 规范化即可对齐。
pub(crate) fn normalize_name(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '的')
        .collect()
}

/// 展示标签：本地别名优先，无别名念云名称（架构 §5，PRD OQ1 采纳）。
pub(crate) fn device_label(row: &DeviceRow) -> &str {
    let alias = row.alias.trim();
    if alias.is_empty() {
        row.name.trim()
    } else {
        alias
    }
}

/// 名称匹配（纯函数）：见模块文档「名称匹配算法」。
pub(crate) fn match_devices(raw_query: &str, devices: &[DeviceRow]) -> NameMatch {
    let query = normalize_name(raw_query);
    if query.chars().count() < 2 {
        return NameMatch::NotFound;
    }
    // ① 精确：alias == query 优先；alias 无命中再看 name == query。
    //    alias 命中即短路（即便 name 还能精确命中别的设备），体现用户起的名字优先。
    for key in [alias_of, name_of] {
        let mut exact: Vec<DeviceRow> = devices
            .iter()
            .filter(|row| !key(row).is_empty() && key(row) == query)
            .cloned()
            .collect();
        match exact.len() {
            0 => continue,
            1 => return NameMatch::Unique(exact.remove(0)),
            _ => return NameMatch::Ambiguous(exact),
        }
    }
    // ② 双向包含：query ⊆ 名（话短名长）或 名 ⊆ query（话长名短）。
    //    离线设备同样参与匹配（架构 §2 ③）。
    let contains: Vec<DeviceRow> = devices
        .iter()
        .filter(|row| {
            [alias_of(row), name_of(row)]
                .iter()
                .any(|name| !name.is_empty() && (name.contains(&query) || query.contains(name)))
        })
        .cloned()
        .collect();
    match contains.len() {
        0 => NameMatch::NotFound,
        1 => NameMatch::Unique(contains.into_iter().next().expect("len checked")),
        _ => NameMatch::Ambiguous(contains),
    }
}

fn alias_of(row: &DeviceRow) -> String {
    normalize_name(&row.alias)
}

fn name_of(row: &DeviceRow) -> String {
    normalize_name(&row.name)
}

/// 开关 DP 识别（纯函数）：取值为 bool 的 DP，`id=="1"` 优先；
/// 无 "1" 时按数字序→字典序取第一个（保证确定性，便于测试与回归）。
pub(crate) fn find_switch_dp(dps: &HashMap<String, Value>) -> Option<String> {
    if dps.get("1").is_some_and(Value::is_boolean) {
        return Some("1".into());
    }
    let mut bools: Vec<&String> = dps
        .iter()
        .filter(|(_, value)| value.is_boolean())
        .map(|(id, _)| id)
        .collect();
    bools.sort_by(|a, b| {
        (a.parse::<u32>().ok(), a.as_str()).cmp(&(b.parse::<u32>().ok(), b.as_str()))
    });
    bools.first().map(|id| (*id).clone())
}

// ============================================================
// 话术组装（纯函数；全集见架构 §5，不做内部 advice 透传）
// ============================================================

/// 成功确认：念命中别名（无别名念云名称）。
pub(crate) fn speech_success(on: bool, label: &str) -> String {
    if on {
        format!("已打开{label}。")
    } else {
        format!("已关闭{label}。")
    }
}

/// 未找到。
pub(crate) fn speech_not_found(name: &str) -> String {
    format!(
        "没有找到叫「{}」的设备，可以在管理台确认设备名称或别名后再试。",
        name.trim()
    )
}

/// 歧义：最多列 3 个标签，请用户说得更具体些（OQ2 采纳：不自动选择）。
pub(crate) fn speech_ambiguous(name: &str, matched: &[DeviceRow]) -> String {
    let labels: Vec<String> = matched
        .iter()
        .take(AMBIGUOUS_LIMIT)
        .map(device_label)
        .map(str::to_string)
        .collect();
    format!(
        "找到多台匹配「{}」的设备：{}。请说得更具体些。",
        name.trim(),
        labels.join("、")
    )
}

/// 控制失败三归类 → 话术（offline/timeout/unsupported 一一映射）。
pub(crate) fn failure_speech(kind: &str, label: &str) -> String {
    match kind {
        "timeout" => format!("{label}没有响应，请稍后再试。"),
        "unsupported" => format!("{label}不是可开关的设备，暂时不支持语音开关。"),
        _ => format!("{label}现在离线，请检查设备供电和网络后再试。"),
    }
}

fn failure_result(kind: &str, label: &str) -> Value {
    json!({"ok": false, "kind": kind, "speech": failure_speech(kind, label)})
}

impl TuyaEngine {
    /// 语音开关一台 Tuya 设备（`tuya.control` 能力入口，infallible）。
    ///
    /// 全流程：目录名称匹配 → 在线预检 → 开关 DP 识别（缓存优先，缺失回读）
    /// → `set_dp` 下发。任何失败都组装成 speech 话术返回，调用方（会话 plan
    /// 分支）拿到 `Ok` 继续播报，会话不中断。
    ///
    /// - `name`：用户说出的设备名/别名（已过 plan 校验：1..256 字符非空）；
    /// - `action`：静态字面量 "on"/"off"（plan 校验层锁死；此处再防御一次）。
    pub async fn voice_control(&self, name: &str, action: &str) -> Value {
        let on = match action {
            "on" => true,
            "off" => false,
            _ => {
                // 校验层已锁死；纯防御（不可达路径也必须有话术）。
                return json!({
                    "ok": false, "kind": "unsupported",
                    "speech": "暂时不支持这个开关操作。"
                });
            }
        };
        let devices = match self.list_devices().await {
            Ok(devices) => devices,
            Err(e) => {
                // 目录读取失败（SQLite 层面）极罕见：不归入「未找到」误导用户。
                tracing::warn!("Tuya 语音控制读目录失败: {e}");
                return json!({
                    "ok": false, "kind": "error",
                    "speech": "暂时读不到设备目录，请稍后再试。"
                });
            }
        };
        match match_devices(name, &devices) {
            NameMatch::NotFound => {
                json!({"ok": false, "kind": "not_found", "speech": speech_not_found(name)})
            }
            NameMatch::Ambiguous(matched) => {
                json!({"ok": false, "kind": "ambiguous", "speech": speech_ambiguous(name, &matched)})
            }
            NameMatch::Unique(row) => self.control_row(row, on).await,
        }
    }

    async fn control_row(&self, row: DeviceRow, on: bool) -> Value {
        let label = device_label(&row).to_string();
        // 在线预检：离线设备直接话术提示（set_dp 内的 online 预检是兜底）。
        if !row.online {
            return failure_result("offline", &label);
        }
        // 开关 DP：缓存优先；缓存 None 先 status 回读填充（架构 §3）。
        let dps = match self.cached_dps(&row.device_id) {
            Some(dps) => dps,
            None => match self.read_dps(&row).await {
                Ok(dps) => dps,
                Err(failure) => {
                    tracing::info!(
                        "Tuya 语音控制 {} 回读状态失败（{}）",
                        row.device_id,
                        failure.kind
                    );
                    return failure_result(failure.kind, &label);
                }
            },
        };
        let Some(dp) = find_switch_dp(&dps) else {
            return failure_result("unsupported", &label);
        };
        match self.set_dp(&row.device_id, &dp, json!(on)).await {
            Ok(_) => {
                json!({"ok": true, "device_id": row.device_id, "speech": speech_success(on, &label)})
            }
            Err(failure) => failure_result(failure.kind, &label),
        }
    }

    /// WASM 控制策略选择候选设备，native 继续执行 DP 识别和 rustuya LAN 控制。
    #[cfg(feature = "wasm-plugins")]
    pub async fn voice_control_with_policy(
        &self,
        host: &crate::plugins::wasm::WasmHost,
        name: &str,
        action: &str,
    ) -> Value {
        let on = match action {
            "on" => true,
            "off" => false,
            _ => {
                return json!({
                    "ok": false,
                    "kind": "unsupported",
                    "speech": "暂时不支持这个开关操作。"
                })
            }
        };
        let devices = match self.list_devices().await {
            Ok(devices) => devices,
            Err(e) => {
                tracing::warn!("Tuya WASM 策略读目录失败: {e}");
                return json!({"ok": false, "kind": "error", "speech": "暂时读不到设备目录，请稍后再试。"});
            }
        };
        let candidates: Vec<Value> = devices
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let has_switch_dp = self
                    .cached_dps(&row.device_id)
                    .is_some_and(|dps| find_switch_dp(&dps).is_some());
                json!({
                    "index": index,
                    "label": device_label(row),
                    "online": row.online,
                    "model": row.model,
                    "has_switch_dp": has_switch_dp,
                })
            })
            .collect();
        let decision = match super::policy::tuya_prepare(host, name, action, &candidates) {
            Ok(decision) => decision,
            Err(e) => {
                tracing::warn!("Tuya WASM 策略失败：{e}");
                return json!({"ok": false, "kind": "error", "speech": "Tuya 控制策略暂时不可用，请稍后再试。"});
            }
        };
        let Some(row) = devices.into_iter().nth(decision.candidate_index) else {
            return json!({"ok": false, "kind": "error", "speech": "Tuya 策略选择的设备已失效，请稍后重试。"});
        };
        self.control_row(row, on).await
    }

    /// DP 缓存未命中时的一次性回读（`get_or_build` + `status()`，架构 §3）。
    ///
    /// 成功即回填 dp_cache（与轮询/控制回读同一份缓存）；失败按三归类返回，
    /// 由 [`TuyaEngine::voice_control`] 映射成话术。
    async fn read_dps(&self, row: &DeviceRow) -> Result<HashMap<String, Value>, ControlFailure> {
        let device = self.lan.get_or_build(row).await?;
        self.lan.ensure_listener(&row.device_id);
        let text = lan::with_call_timeout(device.status()).await?;
        let dps = match text {
            Some(text) => super::dps::parse_dps(&text),
            None => HashMap::new(),
        };
        if !dps.is_empty() {
            self.dp_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(row.device_id.clone(), dps.clone());
        }
        Ok(dps)
    }
}
