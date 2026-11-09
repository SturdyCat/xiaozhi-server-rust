//! 会话观察者挂钩（observe_hook）全路径回归：未装载只告警不 panic、trap 夹具
//! 超预算降级——旁路观察者永不打断对话轮。宿主派发原语（try_dispatch /
//! spawn_deferred）语义回归在 `plugins/wasm/tests/dispatch.rs`。

#[cfg(all(test, feature = "wasm-plugins"))]
mod wasm_observers {
    use crate::plugins::wasm::tests::{ensure_fixture, host_with, ScriptedFetcher};
    use std::sync::Arc;

    #[tokio::test]
    async fn observe_hook_full_path_never_panics_on_unloaded() {
        let fetcher = Arc::new(ScriptedFetcher::new());
        let host = host_with(&fetcher);
        // 未装载插件：observe_hook 只告警不 panic（旁路永不打断）
        super::super::hooks::observe_hook(&host, "ghost.plugin", "start", "{}");
        // trap 插件装载后派发超预算 → 降级告警，同样不 panic
        host.load("trap.hostile", &ensure_fixture("trap"))
            .expect("load");
        // trap 夹具只对 spin 作恶（死循环→fuel 陷阱）——观察者路径同样触发降级
        super::super::hooks::observe_hook(&host, "trap.hostile", "spin", "{}");
        assert_eq!(
            host.snapshot_json()["plugins"][0]["phase"]["state"],
            "degraded"
        );
    }
}
