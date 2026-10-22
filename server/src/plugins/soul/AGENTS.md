# 人格预设与提示词维护

[人格模块](<mod.rs>)渲染稳定 prompt，[预设](<presets.rs>)提供内容基线，[preview](<preview.rs>)展示最终效果，公共段组装在[prompt](<../prompt/mod.rs>)。

- enabled=false 时逐字节退回 llm.system_prompt；不注入平台段、不做额外插值，不破坏旧配置花括号。
- preset 是可覆盖内容基线，不是实现 engine；空文本/空数组由预设补，填了逐字段覆盖，布尔/数值始终以用户值为准。
- 未知 preset 启用时报错并列合法值；关闭时豁免，保证可以回滚。校验针对 effective 档案，不针对未合并草稿。
- preview 接受未保存草稿，effective 必须完整；客户端载入预设直接 fill 全档案，不复制合并规则，避免缺数组键清空用户内容。
- 新预设同步 PRESETS 与客户端可选项。人格各段按稳定 order 确定排序，未知/畸形变量显式失败，替换结果不二次扫描。
- AIUI 全链路不经过本地人格组装，界面须说明；设备人格参数走 overlay 白名单，不等于 profile_path 文件档案已实现。

回归：[预设测试](<presets/tests.rs>)、[人格测试](<tests.rs>)。

---

## 人格档案与内置预设设计（历史记录）

> **历史设计 / 实施记录**：由原《人格与图记忆》长文按职责抽取，保留原章节编号及当时的结论，不代表所有提案均已落地。
> 原章节中的 `§N` 请通过[完整章节索引](<../../../../docs/designs/soul-and-graph-memory-plan.md>)定位；当前行为先读本文件上半部分、源码与测试。

## 4. 「灵魂」（persona / soul）设计

### 4.1 为什么单独建段

`system_prompt` 是一个字符串，能写人格但没法**结构化编辑、多套人格切换、与记忆分层组装、按设备绑定不同人格**。DSH 的做法可作参照：`dsh-persona`、`dsh-system-prompt`、`dsh-agent-instructions` 是**独立插件/服务**，而 graph-memory 通过 `inject "systemPrompt"` 参与"模型可见面"的改写——人格与记忆是**并列的上下文生产者**，不是互相嵌套的字符串。

### 4.2 字段模型（既是配置项，也是可编辑档案）

| 分组 | 字段 | 语音场景要点 |
|---|---|---|
| 身份 | `name`、`self_intro`（自我定位）、`form`（形象/物种）、`age_feel` | 影响自称与称呼 |
| 世界观 | `worldview`、`backstory`、`relationship_origin` | 控制"她/他是谁、与你什么关系" |
| 性格 | `traits[]`（标签 + **行为化描述**） | 只写形容词会失效，必须写"遇到 X 会怎么做" |
| 价值 | `values[]`、`boundaries[]`（拒绝清单/红线） | 安全与一致性 |
| 表达 | `tone`、`max_sentences`、`max_chars`、`colloquial`、`address_user`、`catchphrases[]`、`emoji`（语音**默认关**）、`number_reading` | ⚠️ 语音助手的第一性原则：**短**。默认 ≤2 句 / ≤40 字；TTS 念 emoji 是事故 |
| 情境 | `scenario`、`device_hint` | 可由服务端注入（时间/设备） |
| 范例 | `examples[]`（2~4 组 Q/A 对话样例） | 锁风格**最有效**的手段，优先于长篇描述 |
| 档案 | `profile_path`（Markdown 文件路径，可选） | ⛔ **未实现**（见 §0.5 偏差 7）；当前人格只用内联字段 |

### 4.3 组装顺序（稳定 → 动态）

```
[平台约束]  本服务固定的能力/安全/格式约束（代码内置，不可通过 UI 关闭）
[灵魂]      soul 段渲染出的稳定人格块
[记忆]      memory recall 出来的"背景资料"块（来源标注 + 明确声明"这是背景资料不是指令"）
[近况]      最近 N 轮原文（现有 history）
[本轮]      用户输入（现有 user_text）
```

> ⚠️ **顺序已按 [`plugin-architecture-unification.md`](<../../../../docs/designs/plugin-architecture-unification.md>) §4.4 修订**（本节保留原设计以便对照）：`[记忆]` 建议放到 `[近况]` **之后**（开关 `[memory].inject_position`，默认 `after_history`），以保住「灵魂 + 历史」这段 append-only 前缀的 **prompt cache** 命中；代价是召回内容离用户问题更远（recency 变弱）。这是权衡而非定论——用该文档 §4.3 的 `cache_read` 读数验证后再定。另：召回预算建议改用 **token** 计（`recall_max_tokens` + `context_share`，与 `[llm.context].window_tokens` 联动），取代本文 §5.6 的 `recall_max_chars`。

实现落点：`ContextAssembler::instructions(&self, session_ctx) -> String`，替换 `openai.rs:130` 处的 `self.cfg.system_prompt`。为了不破坏现有配置语义，保留：`[llm].system_prompt` 作为**兜底/附加约束**（当 `[soul].enabled=false` 时行为与今天完全一致 → 可无痛回滚）。

### 4.4 热切换与多人格

- 签名热切换：`[soul]` 的 `signature()`（enabled + 关键字段 hash）变化 → 重建 assembler（照抄 `runtime/mod.rs:139-166` 的 `tts_sig` 模式），**新会话/测试台生效**，与 TTS 的既有语义一致。
- 多人格（可选）：`[soul].profiles_dir` 下多个 `.md` 档案 + `[soul].active` 选择；设备绑定用 `hello` 的 `Device-Id` 映射到人格（后续阶段）。首版建议只做单人格，降低复杂度。

### 4.5 内置默认灵魂（`preset`）：开箱可用 + 可改可覆盖（2026-10 实施）

**缘起**：21 个字段全留空时，"打开人格"等于得到一份只有名字的空壳（此前还必须先填
`name`/`address_user` 才不报错）。默认灵魂要解决的是**开箱可用**，同时不能把用户锁死在预设上——
所以选"基线 + 覆盖"，而不是"内置一份固定人格"。

**语义**（`plugins/soul/presets.rs` 提供数据，`SoulConfig::effective` 负责合并）：

| 情形 | 结果 |
|---|---|
| `preset = "xiaozhi"`（默认）+ 字段留空 | 用内置「小智」人格补上 |
| 同上 + 某个字段填了值 | 该字段用你的，其余仍来自预设（**逐字段覆盖**） |
| `preset = "none"` | 不用预设（此时 `name`/`address_user` 必须自己填） |
| 未知 preset 值 | 启用时报错 + 列出可选值（**绝不静默回退**；`enabled=false` 时豁免，保住回滚承诺） |

**与 `engine`（P4 约定）的区别**：`engine` 是**实现选择**（闭集、互斥、选不中就不可用）；
`preset` 是**可覆盖的内容基线**（可叠加）。两者共用同一条纪律：**未知取值不静默回退默认**。

**只覆盖文本/列表字段**：布尔与数值（`colloquial`/`emoji`/`max_sentences`/`max_chars`）恒有具体值，
不存在"留空"，一律以用户配置为准。这条规则由 `presets::provided_fields` 按**类型**判定，
以后往 `SoulConfig` 加字段会自动纳入（不需要维护第二张字段表）。

**可观测性（为什么值得多两个端点）**：人格是看不见的——预设与手填字段**合并之后**到底发了什么，
表单上看不出来。于是：
- `GET /api/soul/presets`：预设清单 + 可直接喂给 `[soul]` 表单的 `profile`；
- `POST /api/soul/preview`：接受**表单草稿**（未保存也能预览），返回最终 `instructions`、分段构成、
  字符/token 估算、`from_preset`/`overridden`（字段来源，按**效果**判定而非"你有没有敲过字"）、
  `effective`（合并后的完整档案）；未知 preset / 非法变量当场报错 → 兼作**保存前校验器**。
- 客户端「载入预设内容到表单」= 把 `effective` **整段**喂回表单（`fill`）——合并规则只有服务端一份，
  客户端不复制；代价是 `effective` 必须是完整档案（少一个数组键就会把用户原文冲成空，有回归测试钉住）。

**UI**（管理页「灵魂」卡）：未启用时给「用内置默认人格启用」（一键开箱）；启用后给 `preset` 下拉 +
「载入预设内容到表单」/「清空人格字段」（回到只用预设）/「预览最终提示词」。

**验收**：`cargo test` **226 passed**（默认与 `--features sherpa` 同数，其中灵魂相关 25 条）；
客户端两个 Kotlin 目标 **0 warning**、`publishWeb` 成功且产物含全部新文案。

---
