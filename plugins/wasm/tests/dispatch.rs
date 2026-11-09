//! P2d 观察者挂钩的**宿主派发语义**：try_dispatch 只收集不执行、业务性拒绝 =
//! Ok(None)、spawn_deferred 异步落 kv。会话侧 observe_hook 全路径回归在
//! `workflow/session/hooks_tests.rs`（阶段二收口后 hooks 归位会话层）。

use super::{ensure_fixture, host_with, ScriptedFetcher};
use std::sync::Arc;

#[test]
fn try_dispatch_returns_effects_without_executing() {
    // hello 才有 hook 事件；装载 hello 做派发语义测试
    let host = host_with(&Arc::new(ScriptedFetcher::new()));
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load");

    let effects = host
        .try_dispatch("hello.official", "hook.start", r#"{"session_id":"s1"}"#)
        .expect("派发")
        .expect("应有 kv-put 效果");
    assert_eq!(effects.len(), 1);
    // 关键语义：效果未执行——kv 无值（spawn_deferred 才落）
    assert!(host.kv().get("hello.official", "hook-last").is_none());

    // 业务性拒绝（未知事件）= Ok(None)，不算错误、不降级
    assert!(host
        .try_dispatch("hello.official", "nope", "")
        .expect("业务拒绝不是错误")
        .is_none());
    assert_eq!(
        host.snapshot_json()["plugins"][0]["phase"]["state"],
        "active"
    );
}

#[tokio::test]
async fn spawn_deferred_executes_effects_off_turn_path() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load");

    let effects = host
        .try_dispatch("hello.official", "hook.link.nlu.llm", r#"{"text":"杭州"}"#)
        .expect("派发")
        .expect("效果");
    host.spawn_deferred("hello.official", effects);

    // fire-and-forget：有界等待异步落 kv（不 sleep 死等）
    for _ in 0..100 {
        if host.kv().get("hello.official", "hook-last").is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let stored = host
        .kv()
        .get("hello.official", "hook-last")
        .expect("异步执行应落 kv");
    assert!(String::from_utf8_lossy(&stored).contains("杭州"));
}
