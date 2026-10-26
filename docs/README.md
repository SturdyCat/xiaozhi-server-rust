# 文档索引与维护规则

## 阅读入口与当前事实

根目录的 `docs/` 归档项目级概览、跨模块计划与历史长文；除根 `docs/`（含其子目录）与根 `README.md` 外，其他目录只保留该级 `AGENTS.md`，普通 Markdown 不再靠近源码。索引可以跨多级直达目标，不要求沿直属层级逐级跳转。

| 主题 | 阅读入口 |
|---|---|
| 项目介绍与启动 | [项目 README](<../README.md>) |
| 当前跨模块架构 | [架构概览](<architecture.md>) |
| Docker 部署、模型与构建限制 | [部署说明](<deployment.md>) |
| 服务端与客户端维护 | [服务端](<../server/AGENTS.md>)、[客户端](<../client/AGENTS.md>) |
| 配置来源、存储、预设与设备 overlay | [配置说明](<../server/src/config/AGENTS.md>)；SQLite 迁移提案见[存储说明](<../server/src/config/store/AGENTS.md>) |
| 协议与音频下行 | [协议](<../server/src/workflow/protocol/protocol/AGENTS.md>)、[下行](<../server/src/workflow/downlink/AGENTS.md>) |
| EchoMuse 设备入口（Echo Dot 2 / emOS 直连） | [EchoMuse 说明](<echomuse.md>)、[模块指引](<../server/src/device/echomuse/AGENTS.md>)、[手动安装 emOS](<emos-manual-install.md>)（脚本 `tools/emos_install.py`）、[上游 Dot 2 交互/控制文档整理](<echomuse/>) |
| 会话、推理闸门与分段延迟 | [会话](<../server/src/workflow/session/AGENTS.md>)、[推理](<../server/src/workflow/pipeline/inference/AGENTS.md>)、[指标](<../server/src/observability/metrics/AGENTS.md>) |
| HTTP、配置更新与观测接口 | [网关](<../server/src/gateway/ws/AGENTS.md>)、[配置端点](<../server/src/gateway/ws/config/AGENTS.md>) |
| 插件、注册表与能力维护 | [插件层](<../server/src/plugins/AGENTS.md>)（含[注册表](<../server/src/plugins/registry/AGENTS.md>)、[LLM](<../server/src/plugins/llm/AGENTS.md>)、[TTS](<../server/src/plugins/tts/AGENTS.md>)） |
| 人格与记忆 | [人格](<../server/src/plugins/soul/AGENTS.md>)、[记忆](<../server/src/plugins/memory/AGENTS.md>) |
| MCP、OTA、音色探测与文件管理 | [MCP](<../server/src/workflow/mcp/AGENTS.md>)、[OTA](<../server/src/device/firmware/AGENTS.md>)、[音色](<../server/src/device/voices/AGENTS.md>)、[文件管理](<../server/src/gateway/files/AGENTS.md>) |
| 语音主流程、节点测试与公共 SVG UI | [重组与验证记录](<voice-pipeline.md>)、[节点测试样本](<node-test-samples.md>) |
| 公共管理台与 Mac Catalyst 测试台 | [管理台](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md>)、[Mac Catalyst](<../client/apps/macosApp/AGENTS.md>) |
| 测试与回归 | [测试入口](<../server/tests/AGENTS.md>) |

当前行为以源码、测试和邻近维护说明为准。字段与默认值核对对应类型和[配置样例](<../server/config.example.toml>)；热生效档位由注册表元数据唯一声明，不在文档中另维护复制表。

## 文档目录结构

根 `docs/` 只放**当前说明**；历史资料按性质进入三个子目录，子目录内部同样按主题组织、不平铺：

```
docs/
├── README.md                 索引与维护规则（本文件）
├── architecture.md           当前跨模块架构
├── deployment.md             Docker 部署与模型准备
├── echomuse.md               EchoMuse 设备入口（Echo Dot 2 / emOS）
├── emos-manual-install.md    Echo Dot 2 手动安装 emOS
├── node-test-samples.md      节点测试：快照、输入与样本
├── voice-pipeline.md         语音主流程重组与验证记录
├── echomuse/                 上游 EchoMuse Dot 2 交互/控制文档（中文整理版）
├── plans/                    计划、路线与轮次设计（历史交付记录）
├── designs/                  功能设计长文（历史）
└── research/                 调研、评估与分析（过期提案归档）
```

### plans/ 计划与轮次设计

| 文档 | 内容 |
|---|---|
| [总体实施计划](<plans/overall-implementation-plan.md>) | 两路工作合并的分阶段路线图与 Phase 0 交付记录 |
| [P1 增量 PRD](<plans/p1-features-prd.md>) | P1-5 / P1-6 / P1-7 / P1-9 / P1-10 需求与验收 |
| [P1 第一轮设计](<plans/p1-round1-design.md>) | 引擎健康自检 + 记忆多后端注册表 |
| [第 2 轮设计](<plans/p2-round2-design.md>) | MCP 清单驱动动作模型 + 本地 RAG 后端 |
| [第 3 轮设计](<plans/p3-round3-design.md>) | 配置预设 + 每设备 overlay |
| [ESP 兼容与延迟计划](<plans/esp-compat-and-latency-plan.md>) | 整条语音流水线延迟与性能优化方案 |
| [server 模块重构方案](<plans/server-module-refactor-plan.md>) | 按业务功能模块重组 `server/src` 的决策记录（已实施） |

### designs/ 功能设计长文

| 文档 | 内容 |
|---|---|
| [插件架构统一](<designs/plugin-architecture-unification.md>) | 改造前盘点、描述符与宿主、LLM 护栏、配置约定、管理页与分期实施 |
| [人格与图记忆](<designs/soul-and-graph-memory-plan.md>) | 集成背景、上游尽调、人格档案、记忆数据流、界面与提示词调研 |
| [管理台界面设计与验收记录](<designs/admin-ui-design.md>) | 管理页信息架构、配置与上下文页面、健康和记忆的前端验收 |
| [三项缺口设计](<designs/gap-implementation-design.md>) | 推理并发闸门、TTFA 分段指标、残句定时 flush 的设计与实施记录 |
| [TinyBERT NLU 训练流水线](<designs/tinybert-nlu-train-pipeline.md>) | 配置界面一键重训模型方案（设计稿，未实施） |

### research/ 调研与评估（过期提案归档）

以下为**未被采纳的提案**的调研与评估记录，保留证据链与取舍；当前能力以源码、测试与各模块 `AGENTS.md` 为准。

| 文档 | 内容 |
|---|---|
| [架构提案评估](<research/arch-proposal-review.md>) | 差距核实 + 价值审查合并归档；结论：可作设计参考，不作实施清单 |
| [Wyoming Provider 调研与评估](<research/wyoming-provider.md>) | 协议事实卡 + 需求清单 + 可行性评估合并归档；结论：未采纳 |
| [MQTT+UDP Transport 评估](<research/mqtt-udp-transport-assessment.md>) | 需求真实性与接入成本；结论：现在不做 |
| [架构优化分析](<research/architecture-optimization-analysis.md>) | 参考 Golang 版的分模块优化分析（总体计划的上游输入） |
| [代码简化分析](<research/code-simplification-analysis.md>) | server + client/shared 分模块简化分析报告 |

## 文档维护规则

1. **不要为了拆分而拆分。** 同一范围的说明与长文合计不超过 **400 行**时保留在一个文档里；更长的设计、历史与跨模块记录统一归档到根 `docs/`（当前说明留根层，历史资料进 `plans/` `designs/` `research/`），由相关 `AGENTS.md` 直接引用。跨模块接线、取舍和交付记录保留完整语境。**每个目录的维护指引必须是该目录唯一的 `AGENTS.md`**，`docs/` 之外的目录不出现其他普通 Markdown，也不写只有"见链接"的纯导航文件。
2. **仅根目录保留 `docs/`。** 普通 Markdown 只允许出现在根 `docs/`（含其子目录）与根 `README.md`；`docs/` 之外的目录（技能库、第三方依赖与构建产物除外）不新建普通 Markdown 或模块 `docs/`，只保留该级 `AGENTS.md`。
3. **维护指引统一使用大写 `AGENTS.md`。** 根目录 README 保留项目介绍与使用入口；各级指引维护适用范围内的职责和约束，可以直接引用跨多级的相关说明，不重复堆积已经有权威出处的细节。
4. **同一事实只保留一处权威。** 字段与默认值在类型和样例，协议字节布局在源码与测试，热生效在注册表，维护约束在对应指引；其他资料以链接解释关联。行为改变时同步更新当前说明，不把历史提案改写成已实现事实。
5. **迁移保留正文与可追溯性。** 移动或合并时重算相对链接，保留有价值的背景、取舍和验收记录；同主题的多份记录合并时在文件头注明来源与状态。引用旧 `§N` 时说明原章节范围，实际跳转使用目标文件链接。
6. **历史记录明确时点与边界。** 不把旧行号、文件计数、编译 warning 快照、过期路径或历史提交安排当作现行规则；勘误和未落地事项保留，当前状态重新核对源码与测试。过期提案文档在标题下标注状态与结论一句话。
7. **控制修改范围。** 不覆盖无关工作区变更，不自动 `git add` 或 `git commit`；文档整理不借机改动业务行为。

技能库、第三方依赖和构建产物按自身格式维护，不纳入项目说明改名或布局规则。
