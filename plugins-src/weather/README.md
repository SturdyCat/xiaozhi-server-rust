# weather.official — 天气查询（NMC 国家气象中心）

- **插件 id**：`weather.official`（官方默认实现，随内核分发、不可卸载、被回退器兜底）
- **版本**：0.1.0 · **契约**：`xiaozhi:plugin@0.2.0`（api-version `0.2`） · **扩展点**：`skill.weather`（排他）
- **权限**：`kv`、`http-out:www.nmc.cn`

## 功能描述

城市天气查询：城市名解析（含省份消歧）→ NMC 站点码 → 实时天气 / 明天 / 后天 / 近 7 天预报，
产出与原生实现**完全同构**的结果 JSON（`ok/speech/realtime/forecast/hourly/...`）与中文语音话术。
解析与归一化逻辑和原生 `plugins/weather/` 共享同一份 `parse.rs`（单源）。

执行模型是**纯决策瀑布**：本插件不持有任何 I/O——宿主喂事件，插件返回效果（http-request/kv-put），
宿主执行后按步批量回灌 `http.responses`。冷路径 3 次回灌；TTL 内热路径零出网。

## 使用方法

### 事件（宿主 → 插件）

| 事件 | payload（JSON） | 行为 |
|---|---|---|
| `weather.forecast` | `{"req_id":"q1","city":"杭州","province":"浙江","range":"明天"}` | 查询入口；`req_id` 必填，`province`/`range` 可空 |
| `http.responses` | `[{id,status,body,error},...]` | 宿主回灌（勿手工发送）；多步流程上下文编码在各请求 `id` 内 |

结果落到 kv 键 `result:<req_id>`（成功 `ok:true` + 数据；失败也走结果通道：`ok:false` + 话术）。

### 动作（管理台/语音工具，POST /api/plugins/weather.official/actions/\<name\>）

| 动作 | payload | 返回 |
|---|---|---|
| `forecast` | 任意 JSON | 参数提示文案（真正的异步查询走事件通道，动作不做瀑布） |

### 配置字段（`[plugins.weather.official.config]` 段，schema 已注册）

| 字段 | 类型 | 提示 | 说明 |
|---|---|---|---|
| `source` | str | `https://www.nmc.cn` | 数据源根地址（当前为 schema 展示，config-read 导入随 P2 契约扩展生效） |
| `cache_secs` | int | `600` | 结果缓存 TTL 秒（同上） |

### 页面描述（`registration.ui-json`，A2UI 0.1 嵌套组件树）

`register()` 经 SDK `RegistrationBuilder::ui_json` 声明两页（宿主只校验「单个 JSON 对象 +
大小上限」后透传，树由客户端基础组件库渲染）：

| 页 | 树形状 | 绑定 |
|---|---|---|
| `weather-config`（config） | card「数据源与缓存」→ column[field(`source`), field(`cache_secs`)] | `config: [source, cache_secs]` |
| `weather-test`（test） | card「连通性测试」→ column[input(`city`)，actions(["forecast"])，result] | `actions: [forecast]` |

test 页调用 `forecast` 动作时，页面内 `input` 的值以 `{"city":"<输入>"}` 作为动作 payload
（无 input 的页面 payload 为 `{}`，与清单页动作调用同口径）；forecast 动作原样回显参数，正好演示 input→动作→result 闭环。

### kv 键布局（插件隔离命名空间内）

| 键 | 值 | 语义 |
|---|---|---|
| `weather-data:<station>` | `{"ts":<ms>,"data":{...}}` | 按站点码缓存原始天气数据（TTL 600s，结果按 range 现场组装） |
| `wx:<city>:<range>` | `{"ts":<ms>,"result":{...}}` | 结果派生缓存（兼容保留，当前命中以站点原始数据为权威） |
| `station:<city>` | 站点码（如 `58457`） | 城市→站点码缓存（永不过期，避免重复省份扇出） |
| `result:<req_id>` | 结果 JSON | 每次查询的交付物 |

### 请求时序（冷路径）

```
weather.forecast ──► GET /rest/province/all
                  ◄─ http.responses(省列表)
                  ──► GET /rest/province/<code> ×N     （一步并发全部省份）
                  ◄─ http.responses(城市列表 ×N) ── 匹配站点码（kv 缓存）
                  ──► GET /rest/weather?stationid=<code>
                  ◄─ http.responses(天气) ──► 组装结果 + 写缓存 + result:<req_id>
```

### 边界与限制

- 城市为空 → `ok:false`「未指定城市」（geo 默认定位由事件桥 `wasm/bridge.rs` 预填——P2c 已接线）。
- 全站城市列表**不缓存**（kv 单值上限 64KB）；未知城市 → 明确话术报错，不猜近邻。
- 网络/接口失败 → `ok:false` 话术降级，不 panic、不降级插件 phase。
- 城市串内嵌时段词（「杭州明天天气」）自动二次切分；省/市后缀归一化匹配。
- P7a 起缓存粒度按**站点码原始数据**统一（TTL 600s）：同一站点的城市别名和不同 range
  共享一次 HTTP 拉取，结果按本次 range 现场组装，与 native `cached_or_fetch` 对齐。

### 生产接线（P2c）

`[weather].backend = "wasm"`（默认 `native`，改后需重启）即把 `weather.forecast` 技能
切到本插件：skills 层 → `wasm/bridge.rs` 事件桥（扩展点解析 → geo 预填 → 事件派发 →
kv 结果通道）。缺依赖（feature 未编入 / 插件未装载 / 宿主初始化失败）**明确报错，
不静默回退** native——双跑对照的价值在于差异可见。

### 构建与测试

```bash
./build.sh                                          # → server/tests/fixtures/wasm/weather.wasm
cargo test --manifest-path server/Cargo.toml --features wasm-plugins wasm::tests::weather
cargo test --manifest-path server/Cargo.toml --features wasm-plugins wasm::tests::bridge   # 后端开关/事件桥
```

回放测试用脚本化 NMC 应答（零真实网络）：冷路径 4 请求、缓存命中零请求、未知城市优雅失败、
self-test 证明共享 parse 在 wasm 内可用。
