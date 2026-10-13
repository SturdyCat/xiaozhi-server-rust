# xiaozhi-server-rust 整体优化实施方案（合并版）

> 目的：把两路工作合并成一份可执行的分阶段路线图——(A) 前期《分模块优化分析》(`docs/architecture-optimization-analysis.md`，参考 Golang 版优秀设计)；(B) 2026-10 落地的「配置 UI 导航重组」。
> 方法论：取其架构思想，不搬其重实现；凡新增服务端能力优先复用现有 `registry`/`schema`/`bench`/`observe` 基础设施。
> 定位前提：Rust 版 = 离线 ASR/TTS(sherpa-onnx) + 云端 HTTP LLM 的**单二进制 Docker** 小智服务端，固件协议严格对齐(v1/v2/v3)，N5105 低功耗主机；macOS/iOS/Android 有原生麦克风可做语音链路测试，Web 仅能做 LLM/TTS 延迟类轻测。

---

## 0. Phase 0 交付记录（编译验证通过）

### 0.1 配置 UI 导航重组（需求：tab 太多无从下手 → 简化 + 更好组织 + 更友好导航）
- **改动**：14 个平铺 `tabbedPanel` → 「左侧 220px 分组导航 + 顶部搜索框 + 右侧单 section 内容」两栏布局。
- **关键落点**：`ConfigSections.kt`（`group` + `keywords` 中文意图搜索，如"音色"→TTS）；`ConfigFormState.kt`（`configSection` 同步 `configSectionList: ObservableList<String>` + `configSearch`）；`ConfigCards.kt`（`renderConfigLayout`/`renderConfigSection` 复用全部原卡片函数，`vfor({ form.configSectionList })` 保证切换响应式）；`AdminComponents.kt` 的 `tabbedPanel` 保留（TestBench 仍用）。
- **验证**：`:shared:compileKotlinJs` 与 `:shared:compileKotlinMacosArm64` 均 `BUILD SUCCESSFUL`（⚠️ `compileKotlinMetadata` 被 Kuikly 插件 SKIP 是空壳任务，编译验证必须用真实目标）。
- **未做**：**按场景/角色折叠高级字段**——需 schema 配合标记 `advanced` 字段，纯前端无法安全实现；Phase 3「启用开关扩展」可一并评估（见 §4）。

---

## 1. 总体原则与约束

| 维度 | 结论 |
|------|------|
| 离线单二进制定位 | 不引入 Redis 后端、不搬 MQTT-UDP、不对接云端 RAG/MCP 市场/OpenClaw 编排/激活-OTP。 |
| 架构复用 | 新服务端能力优先挂在 `registry`(descriptor)、`schema_json()`、`bench.rs`(测量)、`observe.rs`(埋点) 上，避免另起炉灶。 |
| 跨端边界 | 语音链路测试（麦克风/原生音频）只能 macOS/iOS/Android/OHOS；Web 仅能做 LLM/TTS 延迟类轻测。 |
| 配置系统领先项保留 | schema 驱动 + 热生效三粒度 + 密钥闭环（SECRET_PATHS 三处同改）**不回退**到 Go 的 interface+工厂+build-tag 形态。 |

---

## 2. 分阶段路线图

### ▶ Phase 0 — 配置导航重组 ✅ 已完成（见 §0）
核心价值：降低配置认知负担（分组 + 搜索 + 单 section 聚焦），为后续"向导化/就绪度/内联测试"打底。

---

### ▶ Phase 1 — P0 六项 ✅ 已完成（2026-10 交付）

> ✅ **状态**：P0-1 ~ P0-6 六项全部交付（含 VAD 实例池化 `plugins/vad/pool.rs`、场景化 `mock_client.py`、事件总线 `app/events.rs`）。

> 均契合离线单二进制定位，且多数复用现有代码（`schema_json()` / `bench.rs` / `prefetch` abort 语义 / `mock_client.py`）。

| # | 项 | 维度 | 具体动作 | 落点 |
|---|----|------|----------|------|
| P0-1 | **配置向导化 + 就绪度 + 改动影响高亮** | 功能+UI | 基于 `schema_json()` 自动生成向导式分步表单；`ConfigReadinessState` 按 schema 校验 OTA/VAD/ASR/LLM/TTS 就绪，概览页 + 配置 tab 顶渲染 `readinessBanner`（未就绪模块 + 一键跳转）；首登非阻断 `Coachmark`；复用 `impact{}` 做改动影响高亮 | `commonMain` + 服务端 `app/ws/config.rs` |
| P0-2 | **配置内联测试 + 首包延迟** | UI/后端 | 服务端新增 `POST /api/test/{llm,tts,vad}` HTTP 端点（复用 `bench.rs` 测量，返回 `{state, first_packet_ms, error}`）；配置卡加 `actionRow{secondaryButton("测试", loading)}` + 行内 `textNote`；VAD/ASR 在 Web 禁用 + 灰字提示转测试台；配置 tab 顶栏「测试全部」→ `testSummaryCard` | 服务端 `app/ws/test.rs` + `ConfigFormState`/`ConfigCards.kt` |
| P0-3 | **VAD 实例池化** | 功能 | 实现 VAD 实例池（固定上限 + 空闲超时回收），session 借/还；**前置验证** sherpa-onnx 实例并发约束（线程安全） | `plugins/vad/VadPool` |
| P0-4 | **文本/控制侧优先级变换管线** | 功能 | 轻量优先级 Transformer 管线：指令闸门(wakeup/command) 优先级最高先于 LLM；音频侧 Pacer 不动；复用 `prefetch` abort 语义 | 新增 `app/pipeline.rs` |
| P0-5 | **轻量事件总线解耦** | 功能 | `ServerEvent`(SessionStart/End/MessageAdded/TtsRound/Error) 走 `tokio::sync::broadcast`；session 只 emit，统计/历史/可观测作独立 subscriber（不引 Redis、不引 asaskevich/EventBus） | 新增 `app/events.rs` |
| P0-6 | **场景化联调 + registry 护栏** | 功能 | 扩充 `tests/mock_client.py` 为场景化（多轮 + abort + wakeup + command gate + 记忆召回），CI 必跑；强化「新增 capability 必须注册 descriptor」断言（已有 `tests.rs` 补断言） | `tests/mock_client.py` + `registry/tests.rs` |

**交付校验**：P0-2/P0-3/P0-5 各带单测；P0-6 端到端脚本入 CI。

---

### ▶ Phase 2 — P1 增强 ✅ 已完成（2026-10 交付）

> ✅ **状态（2026-10-13）**：除 P1-8（用户确认剔除）外全部交付。设计记录见 `docs/p1-round1-design.md` / `p2-round2-design.md` / `p3-round3-design.md`，功能需求与验收见 `docs/p1-features-prd.md`。全量回归：`cargo test` 468+ 例全绿、`--features sherpa` 编译通过、Kuikly JS/macOS 双目标编译通过。

| # | 项 | 维度 | 具体动作 |
|---|----|------|----------|
| P1-1 | 延迟监控大盘 | UI/后端 | 服务端聚合首包延迟 `GET /api/metrics`（每模块近 N 轮 VAD/ASR/LLM/TTS 首包 + 整轮 `elapsed_ms` p50/p95 + 错误率，内存态重启清零）；客户端 `renderOpsLatency`，复用 `metricRow/groupedCard/opsHeader` |
| P1-2 | 设备消息注入 | UI/后端 | 服务端代发消息端点 `POST /api/devices/{id}/inject`（server→device 文本/指令，协议本就支持下行文本）；设备卡加「注入消息」`AlertDialog` + 「触发 OTA 检查」；离线设备禁用并提示 |
| P1-3 | 跨端一致性 / 移动端测试台 | UI | 测试台渲染对 Web 做平台分支（Web 显示「测试台不可用 + 前往 macOS/iOS/Android」卡片）；原生逻辑抽可跨原生平台复用 |
| P1-4 | 导航响应式 / 骨架屏 / 分组 | UI | 窄屏(<768) 抽屉式侧边栏（汉堡 + 绝对定位，复用 `ToastHost` 范式）；加载态新增 `ShimmerBlock/SkeletonCard`；`AdminNav` 加 `group` 渲染分组标题 |
| P1-5 | 配置预设 + 每设备 overlay | 功能 | `Config::load` 增 device overlay + preset，不引 Redis |
| P1-6 | 引擎 descriptor 健康自检 | 功能 | 补全 Memory/Soul/Command/Wake 描述符 + `check_all(cfg)->Vec<Phase>` 端点 |
| P1-7 | 记忆多后端注册表 | 功能 | `GraphMemory` 挂 `MemoryProvider` 注册表；新增"纯存储后端"（仅 record 不抽取） |
| P1-8 | ~~声纹 + 动态 TTS 切换~~（**用户 2026-10-12 确认不做，已剔除**） | 功能 | 离线声纹（本地说话人聚类）+ 按说话人切音色 |
| P1-9 | 本地 RAG 后端 | 功能 | 本地文档索引作记忆 trait 可选后端（不对接 Dify 云端） |
| P1-10 | MCP 设备工具清单发现 | 功能 | 握手上报可用 tools → server 缓存按需调用（替代硬编码 `audio_listener.stop`）；command/wakeup 问候统一到"MCP 工具调用结果驱动"动作模型 |

---

### ▶ Phase 3 — P2 按需 / 路线图（默认不投入，待明确需求再评估）

| 项 | 判断 |
|----|------|
| 启用开关扩展（command/memory/soul/mcp） | P2：为可选模块加 `switchRow` 启用开关 |
| OTA 体验强化（版本/回滚/进度） | P2：版本对比/回滚视图、上传进度、矩阵视图（保留现有匹配规则可见） |
| ~~可访问性 / 浅色主题~~（**用户 2026-10-12 确认不做，已剔除**） | P2：补浅色主题、提升 `textTertiary` 对比至 ≥4.5:1、系统应用 `focusRing` |
| MCP 可视化增强 | P2：搜索/筛选/分组折叠、可选调用历史 |
| MQTT-UDP 传输 | P2：**不引入**，保持 WS + v1/v2/v3 回归 |
| OpenClaw / 音乐 / 激活-OTP | P2：不搬（云端编排/增值/本期不做） |
| 第三方 MCP 市场 | P2：不搬（多租户运维平台，离线单二进制不需要） |

---

## 3. 跨阶段风险护栏（贯穿所有 Phase）

1. **密钥护栏**：任何部分更新 + 空串保留密钥改动，必须复用 `SECRET_PATHS` 三处同改护栏 + 回归测试，防"静默清空密钥"。
2. **响应式铁律**：Kuikly 多选一 boolean 不翻转、普通闭包不重渲染——切换类 UI 一律 `vfor({ ObservableList })` 包一层（已在 §0 验证）。
3. **闭枚举扩展**：新增 `Capability`/descriptor 必须全量 match + 护栏测试制衡。
4. **并发约束**：VAD 池化前先确认 sherpa-onnx 实例线程安全（P0-3 前置）。
5. **平台硬限制**：VAD/ASR 无法纯 Web 测试，必须在 UI 上提示降级而非硬上（P0-2/P1-3）。

---

## 4. 下一步

Phase 0 / 1 / 2 均已交付。剩余待办仅两类，均为按需启动：
- **Phase 3（§2）**：默认不投入，待明确需求再逐项评估。
- **折叠高级字段（§0.1 未做项）**：需服务端 schema 增加 `advanced` 标记，可与 Phase 3「启用开关扩展」一并评估。
