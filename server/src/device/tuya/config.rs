//! `[tuya]` 配置段：涂鸦云凭证与同步开关。
//!
//! **热生效档位：全部热生效，无需重启**。凭证（region/access_id/access_secret）由
//! 管理面 `POST /api/tuya/credentials` 写回配置库；worker 每 tick 重读配置，检测到
//! 凭证指纹变化即丢弃 token 缓存（见 `TuyaEngine::check_credentials`）。局域网会话池
//! 与云凭证无关（连接凭据是设备的 local_key），不受凭证变更影响。

use serde::{Deserialize, Serialize};

use crate::config::{canonical_engine, EngineSpec};

/// `[tuya].engine` 可选实现（配置层权威表）。
///
/// `backend`/`lan` 是历史 native 写法，`wasm` 是历史 WASM 策略写法；读取时只做
/// 兼容收敛，序列化与 `engine_id()` 始终使用规范实现 id。
pub const TUYA_ENGINES: &[EngineSpec] = &[
    EngineSpec {
        id: "native",
        aliases: &["backend", "lan"],
    },
    EngineSpec {
        id: "wasm-policy",
        aliases: &["wasm"],
    },
];
const DEFAULT_TUYA_ENGINE: &str = "native";

/// 涂鸦 OpenAPI 支持的区域（`openapi.tuya{cn,us,eu,in}.com`）。
pub const REGIONS: &[&str] = &["cn", "us", "eu", "in"];

/// `[tuya]` 配置段：Tuya IoT 设备集成入口。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TuyaConfig {
    /// 连接器实现：`native`（原生 rustuya）或 `wasm-policy`（WASM 控制策略 + 原生数据面）。
    /// 旧值 `backend` / `lan` → `native`，`wasm` → `wasm-policy`。
    #[serde(default = "default_engine", alias = "backend")]
    pub engine: String,
    /// 总开关。默认 **true**（管理面路由就绪、worker 起跑）；关闭时 `/api/tuya/*`
    /// 统一 404、不轮询、不同步——行为与没有这个模块完全一致。
    /// 局域网控制不依赖云凭证：关掉只影响云目录同步与管理面。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 涂鸦云区域：`cn` / `us` / `eu` / `in` → `openapi.tuya{cn,us,eu,in}.com`。
    #[serde(default = "default_region")]
    pub region: String,
    /// 涂鸦云开发者 access_id（IoT 平台 → 云开发项目）。
    #[serde(default)]
    pub access_id: String,
    /// 涂鸦云开发者 access_secret。**只写不读**：API 层永不返回原值，UI 只见掩码。
    #[serde(default)]
    pub access_secret: String,
    /// 定时云同步周期（秒）。默认 86400（每日一次）；0 = 仅手动同步。
    /// 无论定时还是手动，全局最小同步间隔 60s 闸门都生效（防连点 + 云额度保护）。
    #[serde(default = "default_sync_interval")]
    pub sync_interval_secs: u64,
}

fn default_engine() -> String {
    DEFAULT_TUYA_ENGINE.to_string()
}

fn default_true() -> bool {
    true
}

fn default_region() -> String {
    "cn".to_string()
}

fn default_sync_interval() -> u64 {
    86_400
}

impl Default for TuyaConfig {
    fn default() -> Self {
        TuyaConfig {
            engine: default_engine(),
            enabled: true,
            region: default_region(),
            access_id: String::new(),
            access_secret: String::new(),
            sync_interval_secs: default_sync_interval(),
        }
    }
}

impl TuyaConfig {
    /// 规范化（幂等）：引擎别名/区域白名单收敛 + 同步周期下限（0 = 仅手动，其余 ≥60s）。
    pub fn normalize(&mut self) {
        self.engine = canonical_engine(&self.engine, TUYA_ENGINES, DEFAULT_TUYA_ENGINE);
        self.region = self.region.trim().to_ascii_lowercase();
        if !REGIONS.contains(&self.region.as_str()) {
            tracing::warn!(
                "[tuya].region={:?} 不在支持的区域 {:?} 内，已回落默认 \"cn\"",
                self.region,
                REGIONS
            );
            self.region = default_region();
        }
        self.access_id = self.access_id.trim().to_string();
        self.access_secret = self.access_secret.trim().to_string();
        // 0 = 仅手动；显式配了 >0 的极小值也夹到 60s（与全局闸门同口径，防云额度误伤）
        if self.sync_interval_secs > 0 && self.sync_interval_secs < 60 {
            tracing::warn!(
                "[tuya].sync_interval_secs={} 小于最小同步间隔 60s，已收敛到 60s（0 = 仅手动）",
                self.sync_interval_secs
            );
            self.sync_interval_secs = 60;
        }
    }

    /// 规范化后的实现 id（未知值原样小写，交由注册表 fail-loud 校验）。
    pub fn engine_id(&self) -> String {
        canonical_engine(&self.engine, TUYA_ENGINES, DEFAULT_TUYA_ENGINE)
    }

    /// 凭证是否已配置（id 与 secret 都非空才算——半套凭证无法通过 OpenAPI 鉴权）。
    pub fn credentials_ready(&self) -> bool {
        !self.access_id.is_empty() && !self.access_secret.is_empty()
    }

    /// 凭证指纹（**勿打日志**：含 secret）。热生效检测：指纹变了 → 丢弃 token 缓存。
    pub fn credentials_fingerprint(&self) -> String {
        use std::fmt::Write as _;
        // 不引入额外依赖：FNV-1a 64 位足够做"变更检测"（不用于安全）。
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in format!(
            "{}\u{1f}{}\u{1f}{}",
            self.region, self.access_id, self.access_secret
        )
        .bytes()
        {
            h ^= byte as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let mut out = String::with_capacity(16);
        let _ = write!(out, "{h:016x}");
        out
    }
}

#[cfg(test)]
mod tests;
