# weather.official（NMC 天气官方默认插件）

`skill.weather` 扩展点的官方默认实现（P1c 试点——第一个真实源根插件）。设计见根 [WASM 插件平台](<../../docs/designs/wasm-plugin-platform.md>)；宿主回放测试 `server/src/plugins/wasm/tests/weather.rs`。

## 结构与流程

- 纯决策瀑布：`weather.forecast` 事件 → 效果（http-get/kv-put）→ 宿主批量回灌 `http.responses` → … → `result:<req_id>` 落 kv。冷路径 3 次回灌（省列表 → 一步并发全省城市 → 天气），TTL 内热路径零出网。
- 与原生 `server/src/plugins/weather/` **共享同一份 parse.rs**（`#[path]` 单源；P8 反转为原生从这里取）。解析行为以 `server/src/plugins/weather/tests.rs`（原生）+ 本插件回放测试双证。
- request-id 携带流程上下文（`wx|<code>|<req_id>|<city>|<province>|<range>`，`|` 分隔——约定字段不含 `|`）。
- 刻意不做：全站城市列表不缓存（kv 单值上限 64KB）；`source`/`cache_secs` 字段当前为 schema 展示（config-read 导入随契约扩展后生效）。P7a 起天气原始数据按 `station code` 缓存（TTL 600s），结果按本次 `range` 现场组装；同一站点的城市别名和不同查询时段共享一次 HTTP 拉取，与 native `cached_or_fetch` 粒度一致。
- **生产接线（P2c）**：`[weather].backend = "wasm"` 把技能切到本插件（skills 层经 `server/src/plugins/wasm/bridge.rs` 事件桥：扩展点解析 → geo 预填 → 事件派发 → kv 结果通道）；缺依赖明确报错不静默回退。桥测试 `wasm/tests/bridge.rs`。

## 构建与验证

```bash
./build.sh   # → server/tests/fixtures/wasm/weather.wasm（产物入库）
~/.cargo/bin/cargo test --manifest-path server/Cargo.toml --features wasm-plugins wasm::tests::weather
```
