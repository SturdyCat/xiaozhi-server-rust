//! VAD 实例池（P0-3）：复用昂贵的 Silero ONNX 实例，替代"每会话 `create` 加载模型"。
//!
//! ## 设计（check-out / check-in 独占实例，刻意**不**共享 `Arc`）
//!
//! [`VadEngine`] 只保证 `Send` 不保证 `Sync`（`accept/flush` 均为 `&mut self`，天然要求
//! 独占访问）。因此池化采用**借出所有权**模式：空闲 `VecDeque<Box<dyn VadEngine>>` +
//! `Semaphore` 容量闸门，会话借走一个实例独占使用，`VadGuard` **Drop 时归还**（RAII，
//! 会话正常结束 / `?` 提前返回 / panic 展开都会归还）。多会话并发各持各的实例，
//! 完全规避底层 C++ 对象的线程安全问题（对齐 `downlink`/`prefetch` 的 RAII 范式）。
//!
//! ## 参数失效
//!
//! 借出时对账 `[vad]` 配置签名（`Engines::acquire_vad` 计算）：签名变了就清空池中旧实例
//! 全部重建——保持"改 [vad] 后新会话用新参数"的原语义（`new_vad` 读 `live_config()`）。
//!
//! ## 空闲回收（惰性）
//!
//! 借出前丢弃空闲超过 [`IDLE_TIMEOUT`] 的实例：长闲后池自然清空省内存/线程；
//! ≤1 个常驻实例等效"预热"，首连不付冷启动。不做后台定时任务（P0 最小化）。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::VadEngine;

/// 池上限（同时活跃的 VAD 实例数）。单实例 1 推理线程（`silero.rs` 固定 `num_threads=1`），
/// 8 个 + ASR/TTS 线程预算在 4 核小主机上仍在安全余量内；池满时新会话**等待归还**（背压）。
const MAX_INSTANCES: usize = 8;

/// 空闲超过该时长的实例在下次借出时惰性丢弃（重建比常驻省内存；≤1 个常驻 = 预热）。
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

struct PoolInner {
    /// 当前池内实例对应的 `[vad]` 配置签名（变了即整体失效）。
    sig: String,
    /// 空闲实例 + 归还时刻（惰性回收用）。
    idle: VecDeque<(Box<dyn VadEngine>, Instant)>,
}

struct PoolState {
    inner: Mutex<PoolInner>,
    /// 容量闸门：一个 permit = 一个被借出/新建的实例名额。
    sem: Arc<Semaphore>,
}

/// VAD 实例池。挂在 [`crate::engine::Engines`] 上，跨会话共享。
pub struct VadPool {
    state: Arc<PoolState>,
}

impl VadPool {
    pub fn new() -> Self {
        Self {
            state: Arc::new(PoolState {
                inner: Mutex::new(PoolInner { sig: String::new(), idle: VecDeque::new() }),
                sem: Arc::new(Semaphore::new(MAX_INSTANCES)),
            }),
        }
    }

    /// 借出一个 VAD 实例：优先复用空闲实例；池空且未达上限则 `factory` 新建；
    /// 达上限则等待归还（背压）。`sig` 变化时清空池中旧实例（配置失效语义）。
    pub async fn acquire(
        &self,
        sig: String,
        factory: impl FnOnce() -> Result<Box<dyn VadEngine>>,
    ) -> Result<VadGuard> {
        let permit = self
            .state
            .sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| anyhow::anyhow!("VAD 池信号量已关闭"))?;
        let engine = {
            let mut g = self.state.inner.lock().unwrap_or_else(|e| e.into_inner());
            if g.sig != sig {
                // [vad] 配置变了：池中旧实例参数已过期，整体丢弃重建（新会话用新参数）。
                g.idle.clear();
                g.sig = sig;
            }
            // 惰性空闲回收：借出前丢弃空闲超时的实例。
            while let Some(at) = g.idle.front().map(|(_, t)| *t) {
                if at.elapsed() < IDLE_TIMEOUT {
                    break;
                }
                g.idle.pop_front();
            }
            match g.idle.pop_front() {
                Some((e, _)) => e,
                // 独占锁内不做昂贵 create（sherpa 加载模型可达数秒）：先放锁再建。
                None => {
                    drop(g);
                    factory().context("创建 VAD 实例失败")?
                }
            }
        };
        Ok(VadGuard {
            engine: Some(engine),
            pool: Arc::clone(&self.state),
            _permit: permit,
        })
    }
}

/// 一次借出的 VAD 实例守卫：会话独占使用，**Drop 时自动归还**（RAII）。
///
/// 代理 `accept` / `flush`（与 `Box<dyn VadEngine>` 同名同签名），会话代码零改动。
pub struct VadGuard {
    engine: Option<Box<dyn VadEngine>>,
    pool: Arc<PoolState>,
    _permit: OwnedSemaphorePermit,
}

impl VadGuard {
    pub fn accept(&mut self, samples: &[f32], cb: &mut dyn FnMut(Vec<f32>)) {
        if let Some(e) = self.engine.as_mut() {
            e.accept(samples, cb);
        }
    }

    pub fn flush(&mut self, cb: &mut dyn FnMut(Vec<f32>)) {
        if let Some(e) = self.engine.as_mut() {
            e.flush(cb);
        }
    }
}

impl Drop for VadGuard {
    fn drop(&mut self) {
        if let Some(e) = self.engine.take() {
            let mut g = self.pool.inner.lock().unwrap_or_else(|e| e.into_inner());
            g.idle.push_back((e, Instant::now()));
        }
        // `_permit` 随之 drop → 名额归还信号量
    }
}
