//! 硬件／平台命令的窄接口骨架。
//!
//! 扩展点（本阶段**不实现**）：麦克风采集、Opus 编解码、音频播放、mDNS 发现。
//! 后续新增命令时：
//!   1. 在本模块写 `#[tauri::command]` 函数，保持输入输出为窄数据结构；
//!   2. 在 `src/lib.rs` 的 `generate_handler!` 中注册；
//!   3. 需要系统权限时再显式在 `tauri.conf.json` 引用对应 capability，
//!      并按需启用 `Info.plist` / `Entitlements.plist` 中的预留键；
//!   4. 前端统一通过 `src/lib/platform.ts` 的 `isTauri()` / `getCapabilities()`
//!      做能力检测，Web 环境不得露出硬件动作。
//! 注意：命令可用或页面返回 200 不等于真机硬件通过，硬件结论必须由真机测试给出。

use serde::Serialize;

/// 与前端 `src/lib/platform.ts` 的 `AppCapabilities` 字段严格一致。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCapabilities {
    /// 运行平台（std::env::consts::OS，如 macos / windows / linux）。
    pub platform: String,
    /// 是否运行在 Tauri 桌面壳中。
    pub tauri: bool,
    /// 桌面壳版本（CARGO_PKG_VERSION）。
    pub version: String,
}

/// 示例窄命令：返回桌面壳能力信息，供前端能力检测与测试台展示。
#[tauri::command]
pub fn app_capabilities() -> AppCapabilities {
    AppCapabilities {
        platform: std::env::consts::OS.to_string(),
        tauri: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}
