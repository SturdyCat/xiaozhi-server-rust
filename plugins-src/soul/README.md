# soul.official — 人格风格策略

- **插件 id**：`soul.official`（官方默认实现，`official=true`）
- **版本**：0.1.0 · **契约**：`xiaozhi:plugin@0.2.0`（api-version `0.2`）
- **扩展点**：`prompt.soul`
- **权限**：`kv`
- **定位**：新架构下 server 只做系统底座；本插件承接重构前「人格风格策略」功能的**控制/策略面**。数据面（PCM、模型推理、设备协议、HTTP/SSE）仍由原生宿主持有。

## 功能描述

承接重构前 `[soul]` 人格域的策略面。宿主派发 `prompt.soul.prepare`，本插件读取设备侧传入的语气与预算，按宿主允许的区间做 clamp 后回传；完整 instructions、变量拼装与人格预设 API 仍由原生侧组装。

执行模型遵循插件平台铁律：插件是「纯决策函数 + 效果列表」，不持有任何 I/O 句柄。宿主派发事件 → 本插件同步返回 `Vec<Effect>`（以 `kv-put` 为主）→ 宿主过权限门后异步执行，并从插件隔离 kv 的 `<结果键>:<req_id>` 读回有界 JSON，逐字段做边界校验后交给原生数据面。

## 使用方法

### 事件（宿主 → 插件）

| 事件 | payload（JSON） | 结果键 | 决策内容 |
|---|---|---|---|
| `prompt.soul.prepare` | `{req_id, device_id, name, address_user, tone, max_sentences, max_chars}` | `soul-result:<req_id>` | 回传语气与有界预算 |

`req_id` 由宿主为每次调用生成，必须原样写回结果键；缺失时插件返回业务性错误，宿主显式失败（fail-closed，不静默回退）。

### 动作（管理台 / 语音工具）

| 动作 | payload | 返回 |
|---|---|---|
| `ping` | 任意文本 | 插件 id、扩展点、契约版本与参数回显 |

### 配置字段（`[plugins.soul.official.config]`）

| 字段 | 类型 | 提示 | 说明 |
|---|---|---|---|
| `tone` | str | 温和、简洁 | 人格语气描述 |
| `max_sentences` | int | 2 | 1..=8 |

> 配置字段当前为 schema 展示；`config-read` 导入随契约扩展后生效。

### kv 键布局（插件隔离命名空间内，宿主强制 `<id>:` 前缀）

| 键 | 值 | 语义 |
|---|---|---|
| `soul-result:<req_id>` | 结果 JSON | 本次决策的交付物 |

### 边界与限制

- tone ≤256B、max_sentences ∈1..8、max_chars ∈10..500，本插件已 clamp。
- 当前宿主只定义该桥，生产调用点尚未接线（见验证报告）。

- 插件不持有 PCM / 模型 / 设备句柄；`route` 一律 `native`。
- 默认底座构建未编译对应业务 worker 时，本插件只提供策略决策；业务链路仍需带 `native-plugins` 的镜像。
- 策略结果全部有界，超出宿主校验范围的字段会被拒绝（fail-closed）。

### 构建与分发

```bash
# 单插件
./build.sh                              # → server/tests/fixtures/wasm/soul.wasm
# 全量（读 plugins-src/manifest.toml）
../../tools/build-system-plugins.sh     # → build/system-plugins/soul.official.wasm
```

上传只登记 `Staged`，需依次 `POST /api/plugins/soul.official/register` 与 `POST /api/plugins/soul.official/activate` 才进入 Active；只有 Active 条目参与扩展点解析。
