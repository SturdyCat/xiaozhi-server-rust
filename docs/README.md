# 文档索引与维护规则

## 阅读入口与当前事实

根目录的 `docs/` 归档项目级概览、跨模块计划与历史长文；除根 `docs/` 与根 `README.md` 外，其他目录只保留该级 `AGENTS.md`，普通 Markdown 不再靠近源码。索引可以跨多级直达目标，不要求沿直属层级逐级跳转。

| 主题 | 阅读入口 |
|---|---|
| 项目介绍与启动 | [项目 README](<../README.md>) |
| 当前跨模块架构 | [架构概览](<architecture.md>) |
| Docker 部署、模型与构建限制 | [部署说明](<deployment.md>) |
| 服务端与客户端维护 | [服务端](<../server/AGENTS.md>)、[客户端](<../client/AGENTS.md>) |
| 配置来源、存储、预设与设备 overlay | [配置说明](<../server/src/config/AGENTS.md>)；SQLite 迁移提案见[存储说明](<../server/src/config/store/AGENTS.md>) |
| 协议与音频下行 | [协议](<../server/src/app/protocol/AGENTS.md>)、[下行](<../server/src/app/downlink/AGENTS.md>) |
| 会话、推理闸门与分段延迟 | [会话](<../server/src/app/session/AGENTS.md>)、[推理](<../server/src/app/inference/AGENTS.md>)、[指标](<../server/src/app/metrics/AGENTS.md>) |
| HTTP、配置更新与观测接口 | [网关](<../server/src/app/ws/AGENTS.md>)、[配置端点](<../server/src/app/ws/config/AGENTS.md>) |
| 插件、注册表与能力维护 | [插件层](<../server/src/plugins/AGENTS.md>)（含[注册表](<../server/src/plugins/registry/AGENTS.md>)、[LLM](<../server/src/plugins/llm/AGENTS.md>)、[TTS](<../server/src/plugins/tts/AGENTS.md>)） |
| 人格与记忆 | [人格](<../server/src/plugins/soul/AGENTS.md>)、[记忆](<../server/src/plugins/memory/AGENTS.md>) |
| MCP、OTA 与音色探测 | [MCP](<../server/src/app/mcp/AGENTS.md>)、[OTA](<../server/src/app/firmware/AGENTS.md>)、[音色](<../server/src/app/voices/AGENTS.md>) |
| 公共管理台与 Mac Catalyst 测试台 | [管理台](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md>)、[Mac Catalyst](<../client/apps/macosApp/AGENTS.md>) |
| 测试与回归 | [测试入口](<../server/tests/AGENTS.md>) |

当前行为以源码、测试和邻近维护说明为准。字段与默认值核对对应类型和[配置样例](<../server/config.example.toml>)；热生效档位由注册表元数据唯一声明，不在文档中另维护复制表。

## 历史主题导航

以下入口保留历史主题与原章节范围，正文统一归档在根 `docs/`。设计、实施记录与当前规范须分别阅读：旧行号、文件计数、验收数字和“已完成 / 未开始”仅代表记录时点，不能直接推断当前工作树状态。

- [插件架构统一](<plugin-architecture-unification.md>)：架构盘点、描述符与宿主、LLM 护栏、配置约定、管理页与分期实施。
- [人格与图记忆](<soul-and-graph-memory-plan.md>)：集成背景、上游尽调、人格档案、记忆数据流、界面与提示词调研。
- [管理台界面设计与验收记录](<admin-ui-design.md>)：管理页信息架构、配置与上下文页面、健康和记忆的前端验收。
- [三项缺口设计](<gap-implementation-design.md>)：推理并发闸门、TTFA 分段指标、残句定时 flush，以及跨模块接线与任务护栏。
- [P1 第一轮设计](<p1-round1-design.md>)：健康自检、纯存储后端、接口与数据流、任务和前端验收。
- SQLite 配置迁移提案：[存储说明](<../server/src/config/store/AGENTS.md>)；不是当前 SQLite 配置启动说明。

## 跨模块计划与分析

这些资料保留完整的统筹语境，不因涉及多个模块就机械拆成零碎规则；其中方案和历史交付记录不代替当前使用说明。

| 范围 | 资料 |
|---|---|
| 总体路线与 P1 需求 | [总体实施计划](<overall-implementation-plan.md>)、[P1 增量 PRD](<p1-features-prd.md>) |
| 第二、三轮设计 | [MCP 清单驱动与本地 RAG](<p2-round2-design.md>)、[配置预设与设备 overlay](<p3-round3-design.md>) |
| 架构和代码分析 | [架构优化分析](<architecture-optimization-analysis.md>)、[代码简化分析](<code-simplification-analysis.md>) |
| ESP 兼容与流水线性能 | [ESP 兼容与延迟计划](<esp-compat-and-latency-plan.md>) |

## 文档维护规则

1. **不要为了拆分而拆分。** 同一范围的说明与长文合计不超过 **400 行**时保留在一个文档里；更长的设计、历史与跨模块记录统一归档到根 `docs/`，由相关 `AGENTS.md` 直接引用。跨模块接线、取舍和交付记录保留完整语境。**每个目录的维护指引必须是该目录唯一的 `AGENTS.md`**，根 `docs/` 之外的目录不出现其他普通 Markdown，也不写只有“见链接”的纯导航文件。
2. **仅根目录保留 `docs/`。** 普通 Markdown 只允许出现在根 `docs/` 与根 `README.md`；其余目录（技能库、第三方依赖与构建产物除外）只保留该级 `AGENTS.md`，不新建模块 `docs/`。
3. **维护指引统一使用大写 `AGENTS.md`。** 根目录 README 保留项目介绍与使用入口；各级指引维护适用范围内的职责和约束，可以直接引用跨多级的相关说明，不重复堆积已经有权威出处的细节。
4. **同一事实只保留一处权威。** 字段与默认值在类型和样例，协议字节布局在源码与测试，热生效在注册表，维护约束在对应指引；其他资料以链接解释关联。行为改变时同步更新当前说明，不把历史提案改写成已实现事实。
5. **迁移保留正文与可追溯性。** 移动或合并时重算相对链接，保留有价值的背景、取舍和验收记录；旧主题入口使用简洁语义目录，不为每个原小标题复制一条“见链接”。引用旧 `§N` 时说明原章节范围，实际跳转使用目标文件链接。
6. **历史记录明确时点与边界。** 不把旧行号、文件计数、编译 warning 快照、过期 PLAN 路径或历史提交安排当作现行规则；勘误和未落地事项保留，当前状态重新核对源码与测试。
7. **控制修改范围。** 不覆盖无关工作区变更，不自动 `git add` 或 `git commit`；文档整理不借机改动业务行为。

技能库、第三方依赖和构建产物按自身格式维护，不纳入项目说明改名或布局规则。
