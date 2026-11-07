//! pure-base 下的天气业务兼容接口。
//!
//! 天气查询是业务插件能力；纯底座不编译 NMC 网络客户端，实际查询只能由已装载
//! WASM manifest 提供。这里仅保留配置反序列化和共享装配所需的空接口。

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct WeatherConfig {
    pub engine: String,
    pub source: String,
    pub timeout_secs: u64,
    pub cache_secs: u64,
    pub backend: String,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            engine: "nmc".into(),
            source: "https://www.nmc.cn".into(),
            timeout_secs: 8,
            cache_secs: 600,
            backend: "wasm".into(),
        }
    }
}

pub struct WeatherService {
    backend: String,
    #[cfg(feature = "wasm-plugins")]
    wasm: OnceLock<Arc<crate::plugins::wasm::WasmHost>>,
}

impl WeatherService {
    pub fn new(cfg: &WeatherConfig) -> Self {
        Self {
            backend: cfg.backend.clone(),
            #[cfg(feature = "wasm-plugins")]
            wasm: OnceLock::new(),
        }
    }

    #[cfg(feature = "wasm-plugins")]
    pub fn set_wasm_host(&self, host: Arc<crate::plugins::wasm::WasmHost>) {
        let _ = self.wasm.set(host);
    }

    pub fn forecast(&self, city: &str, province: &str, range: &str) -> serde_json::Value {
        if self.backend != "wasm" {
            return json!({
                "ok": false,
                "speech": "pure-base 天气只能使用已注册的 WASM 插件",
            });
        }
        #[cfg(feature = "wasm-plugins")]
        if let Some(host) = self.wasm.get() {
            return crate::plugins::wasm::bridge::forecast(host, city, province, range);
        }
        let _ = (city, province, range);
        json!({
            "ok": false,
            "speech": "天气后端为 wasm，但天气插件未注册",
        })
    }
}

static WEATHER: OnceLock<Arc<WeatherService>> = OnceLock::new();

pub fn set_global(service: Arc<WeatherService>) {
    let _ = WEATHER.set(service);
}

pub fn global() -> Option<Arc<WeatherService>> {
    WEATHER.get().cloned()
}
