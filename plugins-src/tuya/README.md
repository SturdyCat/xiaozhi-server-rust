# tuya.official — 涂鸦设备控制策略

- **插件 id**：`tuya.official`（官方默认实现，`official=true`）
- **版本**：0.1.0 · **契约**：`xiaozhi:plugin@0.1`（api-version `0.1`）
- **扩展点**：`connector.tuya`
- **权限**：`kv`、`lan-socket`
- **定位**：新架构下 server 只做系统底座；本插件承接重构前「涂鸦设备控制策略」功能的**控制/策略面**。数据面（PCM、模型推理、设备协议、HTTP/SSE）仍由原生宿主持有。

## 功能描述

承接重构前涂鸦 IoT 业务域的控制面。宿主在 `tuya.control` 技能步骤派发 `connector.tuya.prepare`，本插件从候选列表挑第一台在线且带开关 DP 的设备，返回索引与请求动作；另支持 `connector.tuya.status` / `.set-dp` 产生有界 LAN 效果。`local_key`、DP 编码、UDP/TCP 会话与云同步全部由原生连接器持有。

执行模型遵循插件平台铁律：插件是「纯决策函数 + 效果列表」，不持有任何 I/O 句柄。宿主派发事件 → 本插件同步返回 `Vec<Effect>`（以 `kv-put` 为主）→ 宿主过权限门后异步执行，并从插件隔离 kv 的 `<结果键>:<req_id>` 读回有界 JSON，逐字段做边界校验后交给原生数据面。

## 使用方法

### 事件（宿主 → 插件）

| 事件 | payload（JSON） | 结果键 | 决策内容 |
|---|---|---|---|
| `connector.tuya.prepare` | `{req_id, name, action, candidates[]}` | `tuya-result:<req_id>` | 选候选索引 + 回传请求动作 |
| `connector.tuya.status` | `{req_id, device_handle}` | — | 产生 `LanStatus` 效果 |
| `connector.tuya.set-dp` | `{req_id, device_handle, dp, value_json}` | — | 产生 `LanSetDp` 效果 |

`req_id` 由宿主为每次调用生成，必须原样写回结果键；缺失时插件返回业务性错误，宿主显式失败（fail-closed，不静默回退）。

### 动作（管理台 / 语音工具）

| 动作 | payload | 返回 |
|---|---|---|
| `ping` | 任意文本 | 插件 id、扩展点、契约版本与参数回显 |

### 配置字段（`[plugins.tuya.official.config]`）

| 字段 | 类型 | 提示 | 说明 |
|---|---|---|---|
| `prefer_online` | bool | true | 候选选择偏好 |

> 配置字段当前为 schema 展示；`config-read` 导入随契约扩展后生效。

### kv 键布局（插件隔离命名空间内，宿主强制 `<id>:` 前缀）

| 键 | 值 | 语义 |
|---|---|---|
| `tuya-result:<req_id>` | 结果 JSON | 本次决策的交付物 |

### 边界与限制

- `candidate_index` 不得越界、`action` 必须与请求一致；宿主逐项校验。
- `LanStatus`/`LanSetDp` 需要 `lan-socket` 权限，由原生 `TuyaEngine` 消费；pure-base 未编译该引擎时命令只入队不执行。
- 需 `[tuya].engine = "wasm-policy"` 才会派发 `prepare`。

- 插件不持有 PCM / 模型 / 设备句柄；`route` 一律 `native`。
- pure-base 宿主未编译对应业务 worker 时，本插件只提供策略决策；业务链路仍需带 `native-plugins` 的镜像。
- 策略结果全部有界，超出宿主校验范围的字段会被拒绝（fail-closed）。

### 构建与分发

```bash
# 单插件
./build.sh                              # → server/tests/fixtures/wasm/tuya.wasm
# 全量（读 plugins-src/manifest.toml）
../../tools/build-system-plugins.sh     # → build/system-plugins/tuya.official.wasm
```

上传只登记 `Staged`，需依次 `POST /api/plugins/tuya.official/register` 与 `POST /api/plugins/tuya.official/activate` 才进入 Active；只有 Active 条目参与扩展点解析。
