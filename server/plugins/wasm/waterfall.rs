//! 有序事件瀑布（waterfall）+ 强度分级。
//!
//! DSH cordis 的事件纪律直译（`docs/designs/wasm-plugin-platform.md` §概念映射）：
//!
//! - **next() 透传纪律**：不拥有决策权的监听器必须透传，且改写载荷时保留未知字段
//!   （Rust 侧用 `serde_json::Value` 的对象展开近似）；
//! - **强度分级**：restrict < guard < waterfall < replace——契约默认授最弱档，
//!   越强的机制越要保护其他插件的贡献。
//!
//! 本文件是内核通用件（不依赖 wasm），P0 挂在 wasm 模块下，P2 会话层接入时
//! 连同本注释一起移到 `workflow` 下；届时只搬目录不改语义。

use std::collections::BTreeMap;

/// 监听器在瀑布中的强度档位（声明于扩展点契约，越弱越安全）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    /// 只读观察：不能改载荷，任何返回值都被忽略
    Observe,
    /// 否决：可阻断（返回 Err），但不能改写载荷
    Guard,
    /// 改写：可变换载荷后继续传递（须保留未知字段）
    Rewrite,
    /// 独占替换：provider 类扩展点，整体接管
    Replace,
}

/// 瀑布参与条目：同优先级按注册序。数字小先执行。
#[derive(Debug, Clone)]
pub struct Participant<S> {
    pub id: String,
    pub priority: i32,
    pub strength: Strength,
    /// P0：同步决策闭包（wasm 插件调用在 registry 层做桥接，线程池 spawn_blocking 由调用方负责）
    pub handler: S,
}

/// 瀑布执行结果
#[derive(Debug, Clone, PartialEq)]
pub enum WaterfallOutcome {
    /// 全部透传完成
    Completed,
    /// guard 拦截：Err(拦截者 id)
    Blocked(String),
}

/// 通用瀑布：按优先级依次调用 handler。
///
/// handler 签型 `FnMut(&mut P) -> Result<bool, String>`：
/// - `Ok(true)`：载荷被本监听器消费/终结（短路，不再往下传）；
/// - `Ok(false)`：透传（next()）；
/// - `Err(reason)`：guard 否决（仅 Guard/Replace 档允许，Observe/Rewrite 档返回 Err 视为自身故障）。
///
/// 返回 (最终载荷, 结局)。
pub struct Waterfall<S> {
    participants: BTreeMap<(i32, String), Participant<S>>,
}

impl<S> Default for Waterfall<S> {
    fn default() -> Self {
        Self {
            participants: BTreeMap::new(),
        }
    }
}

impl<S> Waterfall<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册参与条目；同 (priority, id) 重复注册即报错（对齐 DSH「重复即报错」）。
    pub fn register(&mut self, p: Participant<S>) -> Result<(), String> {
        let key = (p.priority, p.id.clone());
        if self.participants.contains_key(&key) {
            return Err(format!(
                "waterfall 参与者重复：priority={} id={}",
                p.priority, p.id
            ));
        }
        self.participants.insert(key, p);
        Ok(())
    }

    pub fn unregister(&mut self, id: &str) {
        self.participants.retain(|(_, pid), _| pid != id);
    }

    pub fn len(&self) -> usize {
        self.participants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.participants.is_empty()
    }

    /// 依次执行。`payload` 由调用方定义载荷形状；短路/否决立即返回。
    pub fn run<P, F>(&mut self, payload: &mut P, mut handler: F) -> (WaterfallOutcome, Vec<String>)
    where
        F: FnMut(&mut Participant<S>, &mut P) -> Result<bool, String>,
    {
        let mut trace = Vec::new();
        let keys: Vec<(i32, String)> = self.participants.keys().cloned().collect();
        for key in keys {
            let Some(p) = self.participants.get_mut(&key) else {
                continue;
            };
            match handler(p, payload) {
                Ok(true) => {
                    trace.push(format!("{}:consumed", p.id));
                    return (WaterfallOutcome::Completed, trace);
                }
                Ok(false) => trace.push(format!("{}:next", p.id)),
                Err(reason) => {
                    trace.push(format!("{}:blocked({reason})", p.id));
                    return (WaterfallOutcome::Blocked(p.id.clone()), trace);
                }
            }
        }
        (WaterfallOutcome::Completed, trace)
    }
}
