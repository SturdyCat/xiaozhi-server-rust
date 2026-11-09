//! 宿主服务上下文：插件能触达的**全部通用能力**的聚合根。
//!
//! 设计纪律（用户 2026-11-08：server 构建期不含任何业务）：
//! - 本模块只认识**通用件**（kv / 配置 / 事件总线 / 服务表 / socket / 定时器 / 文件 / mDNS），
//!   不认识任何设备、协议或业务概念；
//! - 插件拿到的是不透明句柄与命名空间，拿不到 fd、路径、凭证；
//! - 会话/设备信息经 [`SystemFacts`] 抽象注入——插件平台不反向依赖会话层实现。

use std::path::PathBuf;
use std::sync::Arc;

use super::config_store::PluginConfigStore;
use super::event_bus::EventBus;
use super::fs::FsSandbox;
use super::kv::KvStore;
use super::mdns::MdnsAdvertiser;
use super::net::NetManager;
use super::services::ServiceTable;
use super::timers::TimerWheel;

/// 会话快照（只读投影；不暴露内部结构）。
#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub id: String,
    pub device_id: String,
    pub phase: String,
    pub started_at_ms: u64,
}

/// 设备快照（只读投影）。
#[derive(Debug, Clone)]
pub struct DeviceSnapshot {
    pub id: String,
    pub online: bool,
    pub last_seen_ms: u64,
    pub client_id: String,
}

/// 服务端事实。
#[derive(Debug, Clone)]
pub struct ServerFacts {
    pub version: String,
    pub data_dir: String,
    pub started_at_ms: u64,
}

/// 会话/设备事实来源。由 server 侧在装配时注入；未注入时为零数据实现。
pub trait SystemFacts: Send + Sync {
    fn sessions(&self) -> Vec<SessionSnapshot>;
    fn session(&self, session_id: &str) -> Option<SessionSnapshot>;
    fn devices(&self) -> Vec<DeviceSnapshot>;
    fn device(&self, device_id: &str) -> Option<DeviceSnapshot>;
    fn server(&self) -> ServerFacts;
}

/// 零数据实现：插件平台独立可测，不依赖会话层。
pub struct EmptySystemFacts {
    pub data_dir: PathBuf,
    pub version: String,
}

impl SystemFacts for EmptySystemFacts {
    fn sessions(&self) -> Vec<SessionSnapshot> {
        Vec::new()
    }

    fn session(&self, _session_id: &str) -> Option<SessionSnapshot> {
        None
    }

    fn devices(&self) -> Vec<DeviceSnapshot> {
        Vec::new()
    }

    fn device(&self, _device_id: &str) -> Option<DeviceSnapshot> {
        None
    }

    fn server(&self) -> ServerFacts {
        ServerFacts {
            version: self.version.clone(),
            data_dir: self.data_dir.to_string_lossy().to_string(),
            started_at_ms: 0,
        }
    }
}

/// 密码学随机源抽象（生产 = OS 熵源；测试/嵌入方可替换实现）。
pub trait RandomSource: Send + Sync {
    /// 填充 `len` 字节随机数（调用方负责长度上限校验）。
    fn fill(&self, len: usize) -> Result<Vec<u8>, String>;
}

/// OS 熵源实现（`getrandom`：Linux getrandom / macOS getentropy / Windows BCrypt）。
pub struct OsRandom;

impl RandomSource for OsRandom {
    fn fill(&self, len: usize) -> Result<Vec<u8>, String> {
        let mut buf = vec![0u8; len];
        getrandom::fill(&mut buf).map_err(|e| format!("OS 随机源不可用：{e}"))?;
        Ok(buf)
    }
}

/// 宿主上下文：所有插件共享同一份。
pub struct HostContext {
    pub kv: Arc<KvStore>,
    pub config: Arc<dyn PluginConfigStore>,
    pub bus: Arc<EventBus>,
    pub services: Arc<ServiceTable>,
    pub net: Arc<NetManager>,
    pub timers: Arc<TimerWheel>,
    pub fs: Arc<FsSandbox>,
    pub mdns: Arc<MdnsAdvertiser>,
    pub facts: Arc<dyn SystemFacts>,
    /// 宿主随机源（WIT `host-core.random`；guest 无熵源时的唯一来源）。
    pub rng: Arc<dyn RandomSource>,
}

impl HostContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kv: Arc<KvStore>,
        config: Arc<dyn PluginConfigStore>,
        bus: Arc<EventBus>,
        services: Arc<ServiceTable>,
        net: Arc<NetManager>,
        timers: Arc<TimerWheel>,
        fs: Arc<FsSandbox>,
        mdns: Arc<MdnsAdvertiser>,
        facts: Arc<dyn SystemFacts>,
    ) -> Arc<Self> {
        Arc::new(Self {
            kv,
            config,
            bus,
            services,
            net,
            timers,
            fs,
            mdns,
            facts,
            rng: Arc::new(OsRandom),
        })
    }
}
