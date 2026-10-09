//! 引擎插件层：ASR / TTS / LLM / VAD / AIUI 全链路，**按能力分目录**。
//!
//! 每个能力目录 = trait 定义 + 配置段 + 各厂商实现文件 + 工厂（`build_*`）。
//! 新增厂商：在对应能力目录加实现文件、实现 trait、工厂加一行注册即可，
//! 会话编排（[`crate::app::session`]）与装配器（[`crate::engine::Engines`]）不动。
//!
//! ## 能力清单（[`registry`] / [`host`]）
//!
//! 上面那句"加一行注册"此前只是**口头承诺**：能力只存在于代码结构里，没有任何地方能
//! 枚举它（没有清单、没有字段元数据、没有状态上报），于是实测新增一个 TTS 供应商要改动
//! **19 处**，且每处调用方都得自己"知道"有哪些实现。现在补上这一层：
//!
//! - [`registry`]：`'static` 常量表 —— 能力 / 实现 / 配置字段元数据 / 热生效语义 / 构建入口
//!   （新增实现真的只需要表里加 1 条）；
//! - [`host`]：装配与校验前移、`enabled` 与 `phase` 分离的状态快照
//!   （`GET /api/plugins`）、必需 vs 可选的分级失败策略。
//!
//! 五个引擎 trait（`AsrEngine`/`TtsEngine`/`LlmProvider`/`VadEngine`/`FullChainEngine`）
//! 在这层引入时**一行未改**——这是本设计能低风险落地的根本原因。
//!
//! 厂商实现文件中的 `#[cfg(feature = "sherpa")]` 仅圈定本地引擎（SenseVoice /
//! Kokoro / Silero）；远程/云厂商实现无 feature 门槛。
//!
//! ## 上下文生产者（P6）
//!
//! [`prompt`] / [`soul`] / [`memory`] 是 LLM **之前**的可插拔上下文生产者：
//! - [`prompt`]：有序 prompt 段注册表（稀疏 order + 启动期变量校验）；
//! - [`soul`]：`[soul]` 人格档案 → 段（关闭时逐字节退回 `[llm].system_prompt`）；
//! - [`memory`]：`[memory]` 图记忆 → 关键路径召回 + 后台异步抽取（默认 `NoopMemory`，零依赖）。
//!
//! 三者的产物经 [`prompt::TurnPrompt`] 交给 LLM（`instructions` = 稳定前缀，
//! 记忆块按 `inject_position` 插入 `input[]`）。
//!
//! ## 指令闸门（`[command]`）
//!
//! [`command`] 是 ASR → LLM **之间**的一道拦截（不是上下文生产者，也调不到 LLM）：
//! 识别文本归一化后按指令词表匹配（`exact` 默认 / `contains` 可选），命中即由会话层
//! 说一句告别语并**断开本次会话**。默认关闭，关闭时零开销、行为与加它之前完全一致。

pub mod aiui;
pub mod asr;
pub mod command;
pub mod host;
pub mod llm;
pub mod memory;
pub mod prompt;
pub mod registry;
pub mod soul;
pub mod tts;
pub mod vad;
