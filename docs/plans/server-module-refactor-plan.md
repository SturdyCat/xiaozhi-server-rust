# server 模块组织重构方案（按业务功能模块 + 消除平铺）

> 状态：**已实施**（2026-10-22）。原 `app/` 已拆为 `workflow/` `device/` `gateway/` `benchmark/` `media/` `observability/` `runtime/` 等顶层业务模块，全局引用与文档链接已同步。本方案保留为重构决策记录。
> 作者：齐活林（交付总监，基于现状分析汇编）
> 日期：2026-10-22

## 1. 背景与动机

当前 `server/src/` 采用**技术分层**组织：

- `app/`：应用层（会话、网关、设备、配置接口、评测、音频、用量……约 30 个模块直接平铺在 `app/` 根）
- `plugins/`：按能力分目录（asr/tts/llm/vad/nlu/memory/soul/skill/command/wakeup/prompt/aiui/health/host/registry）
- `config/`、`runtime/mod.rs`（引擎装配）

**真实痛点不是"分层不对"，而是"平铺"**：

| 目录 | 平铺严重程度 | 说明 |
|---|---|---|
| `app/` 根 | 🔴 严重 | 约 30 个模块文件直接平铺（audio/bench_api/discovery/downlink/echomuse/events/files/firmware/hot_audio/inference/inject/logtime/mcp/metrics/prefetch/protocol/registry/session/splitter/sse/transport/tts_cache/usage/voices/ws） |
| `app/echomuse/` | 🔴 严重 | 约 25 个文件平铺（虽有 `config/` `data_writer/` `media/` 三个子目录） |
| `app/downlink/` | 🟡 中 | `downlink.rs` 在 `app/` 根，子目录 `framer/` `reservoir/` `sender/` 散落 |
| `app/session/` | 🟢 可接受 | 已有 `bench/` 子目录，文件按职责命名清晰；但 `session.rs` 入口与 `session/` 子目录并存 |
| `app/ws/` | 🟢 可接受 | `config/` 已树形；其余 handler 平铺（约 15 个），可整体平移 |
| `plugins/registry/` | 🟡 中 | 10 个文件（`fields_*` 系列 + `impls`/`projection`/`validate`），有结构但可进一步收 `fields/` |
| `plugins/memory/` | 🟢 可接受 | 已有 `store/` 子目录，基本树形 |
| `config/` | 🟢 可接受 | 已有 `store/sqlite/` 子目录 |

用户诉求（已校准）：**不强行套"工作流框架"，核心是消除平铺、按业务功能模块树形组织**；"工作流第一"= 优先把对话编排归类到首位模块。

## 2. 设计原则

1. **拆掉 `app/` 技术伞**：把 `app/` 根的平铺模块按业务语义提升到顶层模块，目录不再大量平铺。
2. **工作流优先**：对话核心闭环（会话状态机 + 流水线环节 + 协议/传输 + 下行）作为第一模块 `workflow/`。
3. **保留已良好的树形**：`plugins/`（按能力分目录，且与 `registry` 强耦合）整体保留；`config/`、`engine`、`ws/config/` 基本保持。
4. **二级拆分消除剩余平铺**：`echomuse/`（约 25 文件）内部再拆 `transport/` `admin/` 等；`plugins/registry/` 的 `fields_*` 收进 `fields/`。
5. **最小语义割裂**：`memory` 因与 `plugins/registry` 的构建/描述符强耦合，**保留在 `plugins/` 内**，不单独提升顶层（否则要改 `registry::impls` 对 `MemoryProvider` 的引用）。

## 3. 目标目录树

```
server/src/
├── main.rs                         # 改 mod 声明：拆掉 mod app; 改为各业务模块声明
├── workflow/                       # 【第一】对话工作流核心
│   ├── mod.rs                      # 重导出会话公共 API（run_session/SessionParams/send_text…）
│   ├── session/                    # 原 app/session（session.rs → session/mod.rs）
│   │   ├── mod.rs  aiui.rs  command.rs  mcp.rs  media.rs  observe.rs
│   │   ├── producer.rs  reply.rs  start.rs  utterance.rs  wakeup.rs
│   │   └── bench/  mcp/  media/  utterance/  wakeup/   # 原有子目录保留
│   ├── pipeline/                   # 流水线环节（原 app 根的编排组件）
│   │   ├── inference.rs  splitter.rs  inject.rs  tts_cache.rs  prefetch.rs
│   ├── protocol/                   # 会话协议与传输抽象（原 app/protocol + app/transport）
│   │   ├── protocol.rs  transport.rs
│   ├── downlink/                   # 下行音频流水线（原 app/downlink*）
│   │   ├── mod.rs  framer/  reservoir/  sender/
│   └── mcp/                        # MCP 客户端框架（原 app/mcp，session 内 MCP 工具探测）
│       ├── mod.rs  discovery/  registry/
├── device/                         # 设备管理
│   ├── mod.rs
│   ├── echomuse/                   # 原 app/echomuse（内部二级拆分，见 §4）
│   ├── firmware/                   # 原 app/firmware
│   ├── discovery.rs                # 原 app/discovery（admin mdns 发现）
│   ├── registry/                   # 原 app/registry（进程内设备/会话注册表）
│   │   ├── mod.rs  types.rs
│   └── voices.rs                   # 原 app/voices（发音人目录服务，有 router）
├── gateway/                        # 管理后台 / 网关接口
│   ├── mod.rs
│   ├── ws/                         # 原 app/ws（整体平移；config/ 子树保留）
│   │   ├── mod.rs  admin_ws.rs  health.rs  log_stream.rs  mcp.rs  memory.rs
│   │   ├── metrics.rs  nlu_build.rs  ota.rs  plugins.rs  registry.rs  soul.rs  usage.rs  bench_api.rs
│   │   └── config/                 # 原 ws/config/*（已树形，保留）
│   ├── files/                      # 原 app/files
│   └── sse.rs                      # 原 app/sse
├── config/                         # 配置（保留现状，已较树形）
│   ├── config.rs  overlay.rs  playback.rs  store.rs  tests.rs
│   └── store/sqlite/
├── benchmark/                      # 评测（原 app/bench_api）
│   ├── mod.rs(capabilities/contracts/features/models/pipeline/providers/samples)
│   └── tests/
├── media/                          # 音频基础设施（原 app/audio）
│   ├── mod.rs  budget.rs  media.rs  opus.rs  pcm.rs  resample.rs  hot_audio.rs
├── observability/                  # 可观测 / 用量 / 事件
│   ├── mod.rs  usage.rs  metrics.rs  logtime.rs  events.rs
├── runtime/                        # 引擎装配 / 共享实例（原 engine）
│   ├── mod.rs(原runtime/mod.rs)  refresh.rs  tts.rs  tests.rs
├── plugins/                        # 引擎能力层（保留，按能力分目录；registry 内部收 fields/）
│   ├── aiui/ asr/ command/ health/ host/ llm/ nlu/ prompt/ skill/ soul/ tts/ vad/ wakeup/
│   ├── memory/                     # 保留（与 registry 强耦合）
│   └── registry/  →  fields/ 收拢 fields_* 子文件
└── tests.rs                        # 集成测试入口（路径引用需更新）
```

## 4. echomuse 内部二级拆分（消除 ~25 平铺文件）

`device/echomuse/` 内部目标：

```
device/echomuse/
├── mod.rs            # 现 echomuse/mod.rs（hub/register/ack/config_push/spawn_mdns）
├── routes.rs         # 现 echomuse/routes.rs（/em/* 设备入口路由）
├── runtime.rs        # 现 echomuse/runtime.rs（LiveControl 等）
├── listen.rs         # 现 echomuse/listen.rs（control 接入监听）
├── media.rs          # 现 echomuse/media.rs
├── transport/        # 传输/帧/蓝牙/上行
│   ├── transport.rs  ble.rs  ble_http.rs  ble_request.rs
│   ├── uplink.rs  frames.rs  turn.rs  data_writer.rs
│   └── *_tests.rs
├── admin/            # 管理面（/api/echomuse/*）
│   ├── admin.rs  system.rs  shell.rs  settings.rs  registry.rs
│   └── *_tests.rs
└── config/           # 现 echomuse/config/*
```

## 5. 文件级迁移清单

> 约定：`→` 左侧为当前路径，右侧为目标路径。`(tests)` 表示对应 `*_tests.rs` / `tests.rs` 一并迁移。

### 5.1 workflow/
| 当前 | 目标 |
|---|---|
| `workflow/session/mod.rs` | `workflow/session/mod.rs` |
| `app/session/{aiui,command,mcp,media,observe,producer,reply,start,utterance,wakeup}.rs` (+tests) | `workflow/session/` 同名 |
| `app/session/bench/*` `app/session/mcp/*` `app/session/media/*` `app/session/utterance/*` `app/session/wakeup/*` | `workflow/session/` 同结构 |
| `workflow/pipeline/inference.rs` (tests) | `workflow/pipeline/inference.rs` |
| `workflow/pipeline/splitter.rs` (tests) | `workflow/pipeline/splitter.rs` |
| `workflow/pipeline/inject.rs` (tests) | `workflow/pipeline/inject.rs` |
| `workflow/pipeline/tts_cache.rs` (tests) | `workflow/pipeline/tts_cache.rs` |
| `workflow/pipeline/prefetch.rs` (tests, bounds, gates) | `workflow/pipeline/prefetch.rs` (+tests/) |
| `workflow/protocol/protocol.rs` (tests) | `workflow/protocol/protocol.rs` |
| `workflow/protocol/transport.rs` (tests) | `workflow/protocol/transport.rs` |
| `workflow/downlink/mod.rs` (tests) | `workflow/downlink/mod.rs` |
| `app/downlink/{framer,reservoir,sender}/*` | `workflow/downlink/{framer,reservoir,sender}/*` |
| `workflow/mcp/mod.rs` `app/mcp/{discovery,registry}/*` `workflow/mcp/tests.rs` | `workflow/mcp/{mod.rs,discovery/,registry/}` |

### 5.2 device/
| 当前 | 目标 |
|---|---|
| `app/echomuse/*`（整体） | `device/echomuse/*`（内部按 §4 二级拆分） |
| `app/firmware/*` | `device/firmware/*` |
| `device/discovery.rs` (tests) | `device/discovery.rs` |
| `app/registry/{mod.rs,types.rs,tests.rs}` | `device/registry/` |
| `device/voices.rs` (tests) | `device/voices.rs` |

### 5.3 gateway/
| 当前 | 目标 |
|---|---|
| `gateway/ws/mod.rs` | `gateway/ws/mod.rs`（router/auth/handshake 保留） |
| `app/ws/{admin_ws,health,log_stream,mcp,memory,metrics,nlu_build,ota,plugins,registry,soul,usage,bench_api}.rs` (+tests) | `gateway/ws/` 同名 |
| `app/ws/config/*` | `gateway/ws/config/*`（保留树形） |
| `app/files/*` | `gateway/files/*` |
| `gateway/sse.rs` (tests) | `gateway/sse.rs` |

### 5.4 其余顶层模块
| 当前 | 目标 |
|---|---|
| `benchmark/mod.rs` `app/bench_api/{capabilities,contracts,features,models,pipeline,providers,samples}.rs` `app/bench_api/tests/*` | `benchmark/{mod.rs,…}` `benchmark/tests/` |
| `app/audio/*` | `media/*` |
| `media/hot_audio.rs` (+tests, tests/media) | `media/hot_audio.rs` (+tests) |
| `observability/usage.rs` (tests) | `observability/usage.rs` |
| `observability/metrics.rs` (tests) | `observability/metrics.rs` |
| `observability/logtime.rs` (tests) | `observability/logtime.rs` |
| `observability/events.rs` (tests) | `observability/events.rs` |
| `runtime/mod.rs` | `runtime/mod.rs` |
| `engine/{refresh,tts}.rs` `runtime/tests.rs` | `runtime/{refresh,tts}.rs` `runtime/tests.rs` |
| `config/*` | `config/*`（保留；可选 `config.rs` → `config/mod.rs`） |
| `plugins/*` | `plugins/*`（保留；`registry` 内 `fields_*` 收进 `registry/fields/`） |
| `src/tests.rs` | `src/tests.rs`（引用路径更新） |

## 6. 影响分析

### 6.1 顶层 mod 声明（`main.rs`）
```rust
// 旧
mod app; mod config; mod engine; mod plugins;
// 新
mod workflow; mod device; mod gateway; mod config;
mod benchmark; mod media; mod observability; mod runtime; mod plugins;
```

### 6.2 全量 import 路径重写（机械替换，影响面最大）
| 旧前缀 | 新前缀 |
|---|---|
| `crate::app::session::` | `crate::workflow::session::` |
| `crate::app::audio::` | `crate::media::` |
| `crate::app::transport::` `crate::app::protocol::` | `crate::workflow::protocol::` |
| `crate::app::downlink::` | `crate::workflow::downlink::` |
| `crate::app::inject::` `crate::app::splitter::` `crate::app::inference::` `crate::app::tts_cache::` `crate::app::prefetch::` | `crate::workflow::pipeline::` |
| `crate::app::mcp::` | `crate::workflow::mcp::` |
| `crate::app::echomuse::` | `crate::device::echomuse::` |
| `crate::app::firmware::` | `crate::device::firmware::` |
| `crate::app::discovery::` | `crate::device::discovery::` |
| `crate::app::registry::` | `crate::device::registry::` |
| `crate::app::voices::` | `crate::device::voices::` |
| `crate::app::ws::` | `crate::gateway::ws::` |
| `crate::app::files::` | `crate::gateway::files::` |
| `crate::app::sse::` | `crate::gateway::sse::` |
| `crate::app::bench_api::` | `crate::benchmark::` |
| `crate::app::usage::` `crate::app::metrics::` `crate::app::logtime::` `crate::app::events::` | `crate::observability::` |
| `crate::engine::` | `crate::runtime::` |
| `crate::app::local_ipv4` 等自由函数 | `crate::device::local_ipv4` 等（按落点调整） |
| `crate::app::(其他)` | 对应新模块（热路径集中在 `main.rs` / `ws/mod.rs`） |

> **注意**：`crate::plugins::memory::` 引用**不变**（memory 留在 plugins）。`crate::config::` 不变。

### 6.3 mod 声明随文件移动
- `session.rs` → `session/mod.rs`：`session/` 子目录的 `mod aiui;` 等声明相对路径不变。
- `runtime/mod.rs` → `runtime/mod.rs`：`engine/` 子目录声明不变（需补 `pub mod refresh; pub mod tts;`）。
- `bench_api.rs` → `benchmark/mod.rs`：声明 `pub mod capabilities;` 等不变。
- `ws/mod.rs` → `gateway/ws/mod.rs`：`mod config;` 指向 `gateway/ws/config/` 不变；其 `use crate::app::…` 需按上表改写。

### 6.4 文档
- `server/AGENTS.md` 与各顶层模块 `AGENTS.md`（workflow/device/gateway/benchmark/media/observability 各建一份；config/plugins/runtime 已有）：更新模块指引与入口链接（已完成）。原 `src/app/AGENTS.md` 的伞式单文件指引取消，职责下沉到各模块自身 `AGENTS.md`。
- `docs/README.md` 架构图：补业务模块视图。

## 7. 风险与缓解

| 风险 | 等级 | 缓解 |
|---|---|---|
| import 路径机械替换漏改导致编译失败 | 中 | 一次性全局 `sed`/批量替换后 `cargo check` 迭代修；优先用 `git mv` 保历史 |
| `crate::app::` 自由函数（如 `local_ipv4`、`json_response`）落点歧义 | 低 | 方案已标注；`cargo check` 会精确报错 |
| `bench_api` 模块名→`benchmark`，跨模块引用（`ws/mod.rs` 的 `bench_api::routes`/`service_router`） | 低 | 替换为 `benchmark::` |
| `echomuse` 内部二级拆分中 `use super::` / `crate::app::echomuse::` 跨文件引用 | 中 | 二级拆分与整体平移合并执行，避免两次改动 |
| `plugins/registry` 与 `memory` 耦合，误把 memory 提顶层 | 已规避 | memory 明确留在 plugins |
| `session.rs` + `session/` 共存的入口机制被破坏 | 低 | `session.rs` 改名 `session/mod.rs`，Rust 解析规则一致 |

## 8. 建议执行步骤（确认后再动手）

1. **骨架**：在 `src/` 下创建 `workflow/ device/ gateway/ benchmark/ media/ observability/ runtime/` 目录与 `mod.rs` 占位；改 `main.rs` 的 `mod` 声明。
2. **平移**：用 `git mv` 按 §5 逐组移动文件（保留 git 历史，便于 review/blame）。
3. **重写引用**：按 §6.2 对全仓 `crate::app::*` / `crate::engine::*` 做机械替换（脚本化）。
4. **二级拆分**：执行 `echomuse/` 内部拆分 + `plugins/registry/fields/` 收拢（可选，可后置）。
5. **编译修复**：`cargo check --manifest-path server/Cargo.toml`（默认 feature）+ `DOCS_RS=1 cargo check --features sherpa` 修残留。
6. **文档**：更新 `server/AGENTS.md` 与相关指引、架构图。
7. **验证**：`cargo test --manifest-path server/Cargo.toml`（默认 feature 单测通过）。

## 9. 开放问题（待你拍板）

1. `memory` 是否坚持提为顶层模块（需同步改 `plugins/registry::impls` 对 `MemoryProvider` 的引用）？当前方案保留在 `plugins/`。
2. `config.rs` 是否改为 `config/mod.rs`（更树形，但改动面略增）？当前保留 `config.rs`。
3. `observability/events.rs`（事件总线）归属：当前放 observability（metrics/日志订阅它）；若你认为它更偏 runtime（引擎事件），可调。
4. 二级拆分（echomuse 内部、registry/fields）是否本次一并做，还是先完成顶层平移、二级后续再做？
