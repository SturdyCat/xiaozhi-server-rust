//! 通用管理面资源变更版本总线。
//!
//! 底座只提供无业务语义的进程级版本 watch。具体插件在注册、卸载或状态变化时可以
//! bump 自己的版本；管理台 WS 只消费通用 `resources_changed` 事件，不知道插件 ID。

use std::sync::OnceLock;
use tokio::sync::watch;

static VERSION: OnceLock<watch::Sender<u64>> = OnceLock::new();

fn sender() -> &'static watch::Sender<u64> {
    VERSION.get_or_init(|| watch::channel(0_u64).0)
}

/// 订阅资源变更版本。发送端进程级常驻，订阅者退出不会影响总线。
pub fn subscribe_version() -> watch::Receiver<u64> {
    sender().subscribe()
}

/// 发布一次通用资源变更。
pub fn bump_version() {
    let tx = sender();
    let _ = tx.send(*tx.borrow() + 1);
}
