//! `server/src/app/inject.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件，
//! 本批清零内联测试块的既存违例）。

use super::*;

#[tokio::test]
async fn send_reaches_registered_session_and_guard_removal_closes_it() {
    let injector = Injector::new();
    let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
    let guard = injector.register("s1", tx);

    injector.send("s1", "你好".into()).expect("注册后可注入");
    assert_eq!(rx.recv().await.as_deref(), Some("你好"));

    // guard 还活着：NoChannel 只对未注册的会话
    assert_eq!(
        injector.send("other", "x".into()),
        Err(InjectError::NoChannel)
    );

    drop(guard);
    // 守卫摘除后通道不在注册表里（NoChannel，而不是 Closed）
    assert_eq!(injector.send("s1", "x".into()), Err(InjectError::NoChannel));
}

#[tokio::test]
async fn closed_receiver_maps_to_closed() {
    let injector = Injector::new();
    let (tx, rx) = mpsc::channel::<String>(CHANNEL_CAPACITY);
    let _guard = injector.register("s1", tx);
    drop(rx); // 会话侧提前结束（理论上 guard 也会同时 drop，这里是防御路径）
    assert_eq!(injector.send("s1", "x".into()), Err(InjectError::Closed));
}

#[tokio::test]
async fn full_channel_maps_to_busy() {
    let injector = Injector::new();
    let (tx, rx) = mpsc::channel(1);
    let _guard = injector.register("s1", tx);
    injector.send("s1", "a".into()).expect("第一条入队");
    assert_eq!(injector.send("s1", "b".into()), Err(InjectError::Busy));
    drop(rx); // 防 unused 警告的读端保活到此刻
}
