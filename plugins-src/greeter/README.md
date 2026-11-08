# greeter.official — 问候演示策略

- **插件 id**：`greeter.official`（官方默认实现，`official=true`）
- **版本**：0.1.0 · **契约**：`xiaozhi:plugin@0.2.0`（api-version `0.2`）
- **扩展点**：`demo.greeter`
- **权限**：`kv`
- **定位**：新架构下 server 只做系统底座；本插件承接重构前「问候演示策略」功能的**控制/策略面**。数据面（PCM、模型推理、设备协议、HTTP/SSE）仍由原生宿主持有。

## 功能描述

对应重构前的演示/连通性验证面，作为 `demo.greeter` 扩展点的独立实现。提供 `demo.echo`（日志回显）与 `demo.kv`（落 kv）两个事件，用于验证「上传 → 注册 → 激活 → 动作」链路；不参与语音主流程。

执行模型遵循插件平台铁律：插件是「纯决策函数 + 效果列表」，不持有任何 I/O 句柄。宿主派发事件 → 本插件同步返回 `Vec<Effect>`（以 `kv-put` 为主）→ 宿主过权限门后异步执行，并从插件隔离 kv 的 `<结果键>:<req_id>` 读回有界 JSON，逐字段做边界校验后交给原生数据面。

## 使用方法

### 事件（宿主 → 插件）

| 事件 | payload（JSON） | 结果键 | 决策内容 |
|---|---|---|---|
| `demo.echo` | 任意文本 | — | 产生 `Log` 效果回显 |
| `demo.kv` | 任意文本 | `last` | 产生 `KvPut` 效果落缓存 |
| `greeter.ping` | `{req_id}` | `greeter-result:<req_id>` | 走统一结果通道的链路冒烟 |

`req_id` 由宿主为每次调用生成，必须原样写回结果键；缺失时插件返回业务性错误，宿主显式失败（fail-closed，不静默回退）。

### 动作（管理台 / 语音工具）

| 动作 | payload | 返回 |
|---|---|---|
| `ping` | 任意文本 | 插件 id、扩展点、契约版本与参数回显 |

### 配置字段（`[plugins.greeter.official.config]`）

| 字段 | 类型 | 提示 | 说明 |
|---|---|---|---|
| `greeting` | str | 你好 | 问候动作的默认称呼 |

> 配置字段当前为 schema 展示；`config-read` 导入随契约扩展后生效。

### kv 键布局（插件隔离命名空间内，宿主强制 `<id>:` 前缀）

| 键 | 值 | 语义 |
|---|---|---|
| `greeter-result:<req_id>` | 结果 JSON | 本次决策的交付物 |

### 边界与限制

- 仅用于链路冒烟，不参与任何生产语音阶段。
- `demo.greeter` 无宿主业务派发点，只有解析表演示与动作调用。

- 插件不持有 PCM / 模型 / 设备句柄；`route` 一律 `native`。
- 默认底座构建未编译对应业务 worker 时，本插件只提供策略决策；业务链路仍需带 `native-plugins` 的镜像。
- 策略结果全部有界，超出宿主校验范围的字段会被拒绝（fail-closed）。

### 构建与分发

```bash
# 单插件
./build.sh                              # → server/tests/fixtures/wasm/greeter.wasm
# 全量（读 plugins-src/manifest.toml）
../../tools/build-system-plugins.sh     # → build/system-plugins/greeter.official.wasm
```

上传只登记 `Staged`，需依次 `POST /api/plugins/greeter.official/register` 与 `POST /api/plugins/greeter.official/activate` 才进入 Active；只有 Active 条目参与扩展点解析。
