# wakeup.official — 唤醒问候策略

- **插件 id**：`wakeup.official`（官方默认实现，`official=true`）
- **版本**：0.1.0 · **契约**：`xiaozhi:plugin@0.2.0`（api-version `0.2`）
- **扩展点**：`wakeup.strategy`
- **权限**：`kv`
- **定位**：新架构下 server 只做系统底座；本插件承接重构前「唤醒问候策略」功能的**控制/策略面**。数据面（PCM、模型推理、设备协议、HTTP/SSE）仍由原生宿主持有。

## 功能描述

承接重构前 `[wakeup]` 唤醒问候域的策略面。宿主派发 `wakeup.prepare`，本插件只返回原生问候目录中的索引，不生成文本或音频；目录长度由宿主校验，空目录时插件明确报错而不是越界。

执行模型遵循插件平台铁律：插件是「纯决策函数 + 效果列表」，不持有任何 I/O 句柄。宿主派发事件 → 本插件同步返回 `Vec<Effect>`（以 `kv-put` 为主）→ 宿主过权限门后异步执行，并从插件隔离 kv 的 `<结果键>:<req_id>` 读回有界 JSON，逐字段做边界校验后交给原生数据面。

## 使用方法

### 事件（宿主 → 插件）

| 事件 | payload（JSON） | 结果键 | 决策内容 |
|---|---|---|---|
| `wakeup.prepare` | `{req_id, greetings[], persist}` | `wakeup-result:<req_id>` | 回传 `{greeting_index, route:"native"}` |

`req_id` 由宿主为每次调用生成，必须原样写回结果键；缺失时插件返回业务性错误，宿主显式失败（fail-closed，不静默回退）。

### 动作（管理台 / 语音工具）

| 动作 | payload | 返回 |
|---|---|---|
| `ping` | 任意文本 | 插件 id、扩展点、契约版本与参数回显 |

### 配置字段（`[plugins.wakeup.official.config]`）

| 字段 | 类型 | 提示 | 说明 |
|---|---|---|---|
| `greeting_index` | int | 0 | 原生目录下标 |

> 配置字段当前为 schema 展示；`config-read` 导入随契约扩展后生效。

### kv 键布局（插件隔离命名空间内，宿主强制 `<id>:` 前缀）

| 键 | 值 | 语义 |
|---|---|---|
| `wakeup-result:<req_id>` | 结果 JSON | 本次决策的交付物 |

### 边界与限制

- `greeting_index` 必须小于目录长度；空目录显式报错。
- 当前宿主只定义该桥，生产调用点尚未接线。

- 插件不持有 PCM / 模型 / 设备句柄；`route` 一律 `native`。
- 默认底座构建未编译对应业务 worker 时，本插件只提供策略决策；业务链路仍需带 `native-plugins` 的镜像。
- 策略结果全部有界，超出宿主校验范围的字段会被拒绝（fail-closed）。

### 构建与分发

```bash
# 单插件
./build.sh                              # → server/tests/fixtures/wasm/wakeup.wasm
# 全量（读 plugins-src/manifest.toml）
../../tools/build-system-plugins.sh     # → build/system-plugins/wakeup.official.wasm
```

上传只登记 `Staged`，需依次 `POST /api/plugins/wakeup.official/register` 与 `POST /api/plugins/wakeup.official/activate` 才进入 Active；只有 Active 条目参与扩展点解析。
