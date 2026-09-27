//! xiaozhi-server-rust 入口。
//!
//! 启动 Axum 服务，加载配置（优先环境变量 `XIAOZHI_CONFIG`，其次 `--config`
//! 参数，否则回退内置 mock 默认配置），初始化共享引擎（ASR/TTS/LLM）。

mod asr;
mod audio;
mod config;
mod engine;
mod error;
mod llm;
mod protocol;
mod session;
mod tts;
mod vad;
mod ws;

use anyhow::Result;
use axum::serve;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::engine::Engines;
use crate::ws::router;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = load_config();
    tracing::info!(
        "配置加载完成：ASR backend={}, TTS backend={}, 监听={}",
        config.asr.backend,
        config.tts.backend,
        config.server.listen
    );

    let engines = Engines::new(&config)?;
    let app = router(engines);

    let listener = TcpListener::bind(&config.server.listen).await?;
    tracing::info!("xiaozhi-server-rust 监听于 {}", config.server.listen);
    serve(listener, app).await?;
    Ok(())
}

/// 配置加载优先级：`XIAOZHI_CONFIG` 环境变量 → `--config` 参数 → 内置默认（mock）。
fn load_config() -> Config {
    if let Ok(p) = std::env::var("XIAOZHI_CONFIG") {
        if let Ok(c) = Config::load(&p) {
            return c;
        }
        tracing::warn!("加载配置文件 {p} 失败，回退默认（mock）配置");
    }
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--config") {
        if let Some(p) = args.get(pos + 1) {
            if let Ok(c) = Config::load(p) {
                return c;
            }
        }
    }
    Config::default()
}
