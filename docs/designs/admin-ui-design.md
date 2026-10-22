# 管理台界面设计与验收记录（历史）

> 定位：本文收录管理页信息架构、配置与上下文页面设计，以及健康和记忆的前端验收记录（原 `config-ui`、`context-ui`、`health-and-memory` 三份记录合并）。
> 内容来源：插件架构统一 §0.11、§5；人格与图记忆 §6–§7；P1 第一轮 §6–§8。
> 当前 UI 约束见[管理台维护](<../../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md>)；文中 `§N` 与 `AGENTS.md §N` 指重组前的旧长文，页面结构以源码与当前指引为准。

---

## 管理页信息架构设计


### 5. 管理页 UI：借"信息架构"，不借"视觉实现"

> DSH 证据来自 asar 内客户端包（`lib/client.js` / `README.md`）。**Kuikly 约束**（`AGENTS.md` §5.8）：只能用官方组件，禁止自造样式。

#### 5.1 DSH 的骨架与两个关键机制（实测）

**顶层骨架**（`dsh-client-ui-layout/lib/client.js` 注册 `root`，`dsh-client-ui-sidebar` 注册其子座）：

```
root (AppFrame)
├─ sidebar          single   ← 侧栏；其下再声明 sidebar.panellist(list) / sidebar.settings(single) ...
├─ main             keyed    ← 主区；会话、插件管理页各占一个 key
├─ rightbar         single
├─ shell.overlay    list     ← toast / 浮层
└─ shell.leading    single
```

**设置面板 4 层**（`dsh-client-ui-settings-general/lib/client.js`）：

```
sidebar.settings [single]  ← 面板本体（800×800 居中 modal）
├─ settings.action   list    ← 标题栏右侧动作
├─ settings.section  list    ← ★ 左侧导航 = 这个 ledger 的投影
└─ settings.onboarding list
settings.section [list]  ← 每个 section 再声明自己的子座
├─ general (order 0)  → settings.general.item [list]
├─ models  (order 10)
└─ plugins (order 15) → settings.plugins.tab [list]
```

实测几何（可作我们排版口径的参考）：面板 800×800、左导航 **188px**（gap 18 / padding 22 12 0）、导航项高 **40px**、标题栏高 **54px**、内容列 `padding: 0 24px 24px` 且**自己滚动**。

**两个关键机制**：

1. **「声明即授权」**（`dsh-client-ui-slots/README.md:46` 原文）：*"Declaring a slot is claiming it: the registering entry becomes the only entry allowed to render that key, and registering into an undeclared slot … throws at load."* → **未声明就注册 = 加载期报错**，不静默丢弃。
2. **排序确定**：`list` 按 `order` 升序、同值按注册顺序；`chain` 按 `priority` 升序、首个非 null 胜出。实测 order：`settings.section` = general 0 / models 10 / plugins 15；`settings.general.item` = developer-tools 15 / **current-version 100（版本号永远在底部）**。

**写页与读页的分工**（这条是我们的直接模板）：
- **侧栏 Plugins 页 = 写**（安装/启停/卸载/配置该插件自己的设置）；
- **Settings → Built-in plugins = 只读清单 + 运行状态**；
- 两页**互相指路**（写页的说明里写"内置插件列表与运行状态见 设置 → 内置插件"）。

#### 5.2 我们的落地骨架（Kuikly 官方组件）

DSH 的"800×800 居中 modal + 遮罩 + 焦点陷阱"在 Kuikly 下**不可实现**（官方只有 `AlertDialog`，无尺寸可定制的模态容器，`AGENTS.md` §5.8）。因此**用整页 + 官方 Tabs+PageList**：

```
┌ 侧栏（统一 @Page("admin") 壳；⚠️ 历史草图：原 AdminShell 双壳已废除，macOS 与 web 同壳，
│        测试台项仅 macOS 显示，见 shared AdminPlatform.kt expect/actual）─┬─ 内容区 ────────┐
│ 测试台                                              │ 能力总览            [刷新]      │
│ 配置                                                │ 共 9 个 · 5 运行中 · 1 降级 · 1 异常 │
│  ├ 总览           ← 新增                            │                               │
│  ├ 语音识别 ASR                                     │ 官方能力                      │
│  ├ 语音合成 TTS                                     │ ┌───────────────────────────┐ │
│  ├ 大模型 LLM                                       │ │ ASR  SenseVoice   运行中 ●│ │
│  ├ 全链路 AIUI    ← 现在完全缺失                    │ │      本地离线 · 新会话生效│ │
│  └ 记忆 / 灵魂    ← soul 文档 P0/P1                 │ ├───────────────────────────┤ │
│                                                     │ │ LLM  OpenAI 兼容  降级 ⚠ │ │
│                                                     │ │      未配 api_key，调用会 401│
│                                                     │ └───────────────────────────┘ │
└─────────────────────────────────────────────────────┴───────────────────────────────┘
一级：总览（清单）→ 二级：某能力 = tabbedPanel(配置 / 状态 / 日志)
```

- **总览页**：`largeTitleBar` + 计数摘要 + `groupedCard("官方能力"/"外部能力")` + `cardRow` 列表；
- **能力详情**：官方 `Tabs + PageList`（`AdminComponents.kt:tabbedPanel`，**必须显式传 `pageItemWidth/Height`**）分「配置 / 状态 / 日志」三个 `TabPage`；
- **导航表**：Kotlin 侧 `data class SectionDescriptor(id, order, label)` 常量表，集中定义、启动期校验 id 唯一与 order 稳定（**不模拟运行时槽位**，见 §5.7）。

#### 5.3 状态呈现规范（照抄 DSH 的"只在偏离时打标"，但**补上原因**）

DSH 的两条规则（`ui-settings-plugin-inventory/README.md:36` 原文）：

> *"A small enablement tag marks only the states that differ from plainly enabled: disabled, conditional, provided by presets, and failed; a plainly enabled row carries no tag."*
> *"A colored root-fiber status dot marks the live phases no tag states: pending maps to idle, while loading and unloading map to ongoing; an active or failed fiber shows no dot."*

实测映射表（`ui-plugin-manager/lib/client.js`）：`PHASE_KEYS = {pending:"等待依赖", loading:"加载中", active:"运行中", failed:"异常", unloading:"卸载中"}`；`phase === null → "未运行"`；`!enabled → "已关闭"`；`PHASE_STATES = {pending:"idle", loading:"ongoing", active:"done", failed:"error", unloading:"ongoing"}`。

**我们的映射（§3.1 的 `Phase` → UI）**：

| 配置意图 | 实际 phase | 徽标 | 点 | 是否显示原因 |
|---|---|---|---|---|
| `enabled=true` | `Active` | 无（正常不打标） | 无 | — |
| `enabled=true` | `Degraded{reason}` | **「降级」** | 黄/`warn` | **必须显示 reason**（如"未配 embedding → 词法模式"） |
| `enabled=true` | `Failed{error}` | **「异常」** | 红/`error` | **必须显示 error + 日志入口** |
| `enabled=true` | `Loading` | 「加载中」 | 蓝/`ongoing` | — |
| `enabled=false` | `Disabled` | 「已关闭」 | 无 | — |
| `enabled=false` | 任何 | 「已关闭」优先 | 无 | — |

⚠️ **这里要明确超过 DSH**：DSH 的 `ui-plugin-manager/README.md:150` 自陈 *"Rows show a phase, not a reason — a failed row reads as failed without the Host's error text; the Host log has it."*（**只有阶段，没有原因**）。我们的 `PluginStatus` 里**已经带上 `last_error` 与 `degraded_reason`**，没有理由不显示——用户看到"异常"却查不到原因，是最容易招致"这功能没用"的形态。
另注意 DSH 的「运行中」与「已启用」是两种信息：**`enabled` 是保存的选择，phase 是实际状态**——这正是 §3.1 把两者分开上报的原因。

#### 5.4 表单规范（六条，全部有 DSH 出处）

1. **草稿 + 单一保存点**：`blocked = !dirty || invalid || saving`；DSH 在卸载时**丢弃草稿且不提供"放弃"按钮**（`ui-primitives/lib/index.js` 注释：*"Leaving the page drops every staged edit, so the form discards on unmount and offers no discard control"*）。我们的 `ConfigPage.kt` 已是"顶栏保存按钮"形态，保持即可。
2. **覆盖语义按"存在性"而非"值比较"**（原文：*"That presence, not a value comparison, is what marks a field overridden: an override equal to the composition default is still an override."*）→ 字段旁给「已覆盖」+「恢复默认」；**空输入 = 清除 = 恢复继承**。这正好对应我们「`config.toml` 覆盖内置默认」的真实语义。
3. **hint 讲后果，help 放展开区**。DSH 文案范例（中文，直接可用）：*"单条命令允许运行多久，超时即终止。"*、*"超出部分会转存到临时文件，而不是被丢弃。"*、*"请填数字；留空表示使用默认值。"*、*"本部署的设置为只读。"*、*"该插件当前未加载，暂时无法配置。"*、*"本部署没有接受这些值，已保留供你修改。"* → **不要复述标签**（"超时时间"→"单条命令允许运行多久"）。Kuikly 无 tooltip，用 `vif` 展开的 `Text`（12px 次级色）作为 help 区。
4. **密钥字段：只报 presence，空草稿保留原值**。DSH 原文：*"The value never rides a response, so the control reports only whether one is configured and starts blank; a blank draft writes nothing, which keeps the stored key rather than clearing it."*；文案 *"不写入设置文件。留空表示保持当前密钥。"* / *"已配置密钥。"* / *"未配置密钥"*；校验 = trim 后非空且**全为可打印 ASCII**（`[\x21-\x7E]`），拒绝 `NAME=value` 与成对引号。服务端侧 *"Secret values cross in this direction only: no method here returns one"* + `redactSecrets: true`。
   → **与 §1.7 bug-1 / §3.4 完全一致**：`GET /api/config` 只回 `has_api_key: true`；`POST` 里**空字符串视为"未提供"**（这条必须显式写进契约，否则旧客户端回传空串会把密钥清掉）。
5. **revision 冲突**：DSH 的 code 是 `settings/conflict`，客户端文案 *"Someone else changed these settings while this card was open. Close it and reopen to edit the current values."*，且**保留草稿**。→ 我们 `POST /api/config` 冲突返回 409 + 该文案花色的中文（"配置在别处被改过，请重新加载后再保存"）+ 保留草稿 + 「重新加载」按钮。
6. **保存后的三档生效文案**（DSH 有 profile 级 HMR 判定与 toast：*"A profile with HMR recomposes before the operation completes; one without HMR, and a bundle a higher layer overrides, say so in a toast."*，文案 `restartNotice="更改将在下次启动生效"`、`overriddenNotice="{name} 已保存，但被更高优先级的配置覆盖，当前未生效"`）：
   - **「已保存并立即生效」** ← 对应 `HotReload::Live`（LLM/TTS/记忆/灵魂）；
   - **「已保存，新会话生效」** ← 对应 `HotReload::NextSession`（VAD 阈值等）；
   - **「已保存，下次启动生效」** ← 对应 `HotReload::RestartOnly`（`server.port` / `worker_threads` / 下行音频参数）。

   ⚠️ **诚实标注**：DSH **没有**字段级"需重启"标注（只有 profile 级 HMR 与 toast），所以 **§3.1 的 `FieldSchema.hot` 字段级三档标记是我们自己的设计**，不是借鉴结果——而它恰好修复了 §1.7 bug-2 那类"文档说生效、实际没生效"的问题。

#### 5.5 动作与长任务（统一现有两套契约）

现状是**两套语义相同却互不兼容**的契约：`voices.rs` 的 `probing` 字段 + 轮询（HTTP），与 bench 的 `tts_test`/`llm_test` 结果帧（WS）。统一为 §3.4 的 action 模型后，UI 只需要一套组件：

| 动作 | 现状 | 统一后 |
|---|---|---|
| 探测音色目录 | `POST /api/tts/voices/refresh` + `GET /api/tts/voices` 轮询 `probing` | `POST /api/plugins/tts/actions/probe-voices` → job id；`GET .../actions/{job}` |
| 测试合成 / 测试识别 / 测试 LLM | WS 的 `tts_test`/`asr_test`/`llm_test`（**且 `asr_test` 把错误当识别文本**，§1.8#4） | `.../actions/test`，结果统一 `{state:"ok"\|"error", message, elapsed_ms, sample?}` |
| 拉取可用模型列表 | 无 | `.../actions/list-models`（§4.7） |
| 清空/导出记忆 | 无 | `.../actions/clear-memory` / `export` |

UI 呈现口径（照 DSH）：手动触发时按钮**禁用 + `ActivityIndicator`，且至少 400ms 防止闪烁**；成功**不弹 toast**；失败保留原内容 + toast；首次加载失败 → 就地错误 + 「重试」；计数摘要**只显示非零项**（`共 N 个 · X 运行中 · Y 异常`，0 的项不出现）。

#### 5.6 破坏性操作

DSH 两档（都在 Kuikly 官方 `AlertDialog` 能力内）：
- **普通确认**：问句标题 + 一句后果 + 危险主按钮（例：*"卸载「{name}」？"* / *"卸载后它提供的功能会消失。"*）。危险色**不是新组件**，而是重绑主按钮的填充色 token；
- **强确认**（`RiskConfirmation`）：标题 + 说明 + **复选框闸门**（未勾选则确认按钮 disabled）。

⚠️ **Kuikly 差异**：官方 `Switch` 是"状态开关"，语义上不能当"我已阅读"的确认控件（会被误读为设置项）。→ 强确认降级为 **AlertDialog 两段式**（第一段讲后果 → 第二段确认），或对极端危险操作要求**输入资源名**匹配（后者是我们的设计，DSH 未找到证据）。
删除文案还要**分叉说明后果**（DSH：删除 provider 时区分"凭据另存所以保留"vs"同时删除已存密钥"）→ 我们"清空记忆"必须明确：**是否同时删除 `db_path` 文件、是否保留导出**。

#### 5.7 明确**不照搬**（Kuikly 官方组件约束）

| DSH 做法 | 为什么不照搬 | 官方等价物 |
|---|---|---|
| 800×800 居中 modal + 遮罩 + 焦点陷阱 | 官方只有 `AlertDialog`，无尺寸可定制模态 | **整页 + `tabbedPanel`**（Tabs+PageList） |
| hover 态 / tooltip / HoverCard / 悬停延迟 | 无官方 tooltip，触屏也无 hover 语义 | **DSH 自己的 help 展开区**（`vif` + `Text`）——最接近的等价物 |
| 自绘滑动指示器 / 选中胶囊 | `AGENTS.md` §5.8 明令禁止 | 官方 `Tabs` + `indicatorInTabItem`（`tabbedPanel` 已封装） |
| 自绘旋转 loader / shimmer 骨架屏 | 无官方 shimmer 与相位控制 | 官方 `ActivityIndicator`；接受相位不对齐 |
| `Tag`/`Pill` 8 套 tone + `color-mix()` 动态淡色 | Kuikly 无 `color-mix` | 预置 tint 常量（`statusBadge` 已这么做）+ **限制 tone 数量** |
| `RiskConfirmation` 的复选框闸门 | 官方 `Switch` 语义不符 | AlertDialog 两段式 / 输入资源名 |
| pnpm 安装日志、registry 探测、包管理细节 | 我们没有 npm/pnpm 域，照搬会**引入无数据来源的 UI** | 不实现（我们的清单来自 `/api/plugins`） |
| 键盘漫游 / aria / `focus-visible` / `inert` / `role="img"`+`title` | 官方组件不暴露这些属性 | **一切信息必须有可见文字**：状态点必须跟文字（如"已配置/未配置"） |
| 0.5px 边框 / `backdrop-filter` / `corner-shape` / `-webkit-line-clamp` | Kuikly 无对应能力 | 固定 1~2 行 + 显式截断策略 |
| **槽位机制本身**（`slots.register/inject`、四类 kind、`declare module` 类型合并、`chain` 竞选） | 本项目 Kuikly 侧**没有运行时 UI 插件加载** | 只借"声明 + 有序注册表 + 加载期校验"的思想 → **Kotlin 编译期常量表**，不模拟 keyed/chain |
| `settings-models` 的模型目录编辑器（Fetch models / input types 复选 / Restore defaults） | 我们配置是 TOML 固定字段，照搬会产生**无法保存的字段** | 只借「测试连接 → 返回模型列表 → 下拉选择」 |

#### 5.8 设计 token 口径（补进 `AdminTheme.kt` 的常量）

DSH 全部走语义 token（`--dsw-alias-*` / `--dsw-radius-*` / `--dsw-elevation-*`，focus ring 2px）。实测可对齐的间距/字号口径：

| 项 | DSH 口径 |
|---|---|
| 字段行 | `padding: 12px 0`，相邻字段 `border-top: .5px` |
| 输入框 | 高 `34px`、`padding: 0 12px`、圆角 `md` |
| 卡片头 | `gap: 14px`、`padding: 8px`；图标 `48px` |
| 标题 / 描述 | 标题 `14/20/500`（单行省略）；描述 `13/18`（1 行截断，三级色） |
| 开关 | `36×20`（thumb 16，位移 16） |
| Tag / Pill | Tag `11/17`、`padding 1px 8px`、radius 999；Pill 高 `24px` |
| 状态点 | 槽 `10px`、实心核 `6px` |
| 左导航 | 宽 `188px`、项高 `40px` |

`AdminTheme.kt` 已有 `AdminColors/AdminShape/AdminType/AdminSpace` 同类抽象 → 按上表补齐常量即可，**不新增组件**。

#### 5.9 兼容与迁移（UI 侧）

- 新增「总览 / AIUI / 记忆 / 灵魂」页面只改两处：`ConfigCards.kt:22-34` 的 `pages` 列表 + 新增卡片函数（若沿用现有 `renderForm` 结构）；有 `SectionDescriptor` 表后收敛为一处；
- **`[aiui]` 卡必须一并补上**（当前 `commonMain` grep `aiui` 零命中，配置段完全不可见，见 §1.8#9）；
- 保存语义改为"部分更新"后（bug-1），**旧客户端（回传完整 JSON 但缺字段）依然安全**；但**密钥打码会破坏旧客户端**（它会把空串回传 → 按 §5.4 规则"空串=未提供"处理即可兼容），因此**打码与空串约定必须同批上线**。

---

### 配置 UI 落地记录（§0.11 历史批次）


#### 0.11 第七批：P5 UI 重组（本文件 §5 的落地）

| 层 | 改动 |
|---|---|
| 服务端 · 三档生效判定 | `POST /api/config` 响应新增 `hot` / `hot_hint` / `changed_sections`。判定在 `gateway/ws/config/impact.rs`：两侧先 `normalize()`（否则"旧位置搬到规范位置"会被误报成真实变更）→ **叶子级** diff → 逐级查注册表 `field_hot`（字段级优先）→ 回退 `section_hot` → 取**最严格**（`HotReload::strictest`）。新增 `registry::{field_hot, section_hot, section_hot_known}` + `HotReload::{rank, strictest}` |
| 服务端 · 非能力段 | `APP_SECTION_HOT`（`[server]`/`[audio]` → `restart_only`）+ `schema_json()` 新增 `app_sections` 投影；`every_config_section_has_declared_hot_semantics` 把"新增配置段忘登记档位"从**静默降级成需重启**变成硬失败 |
| 客户端 · 能力总览页 | 新 tab「总览」（`PluginMetaState.kt` + `ConfigOverviewCard.kt`）：`GET /api/plugins` → 必需/可选两组 + 计数摘要（**只显示非零项**）+ 状态徽标（**只在偏离时打标**）+ **降级/异常原因行内展开** + 「刷新」（失败 → 就地错误 + 可行动指引，不弹 toast） |
| 客户端 · 页签表 | `ConfigSections.kt`：`SectionDescriptor(id, order, label)` 常量表 + 构建期校验（id 唯一 / order 升序 / 非空）；`renderForm` 由表驱动，新增 tab 从"改一行并记住顺序"收敛为"改表 + 加一条 `when`" |
| 客户端 · 三档文案 | `save()` 读响应 `hot` → 「已保存并立即生效 / 已保存，新会话生效 / 已保存，下次启动生效」；服务端 message 里那句"对**新会话**生效"已删除（档位改由字段元数据 + `hot` 回答，不再由文案承诺） |
| 客户端 · 热生效提示 | `FormSchemaState.kt`：`GET /api/config/schema` → 段级 `sectionNote`（Server/Audio/AIUI 卡各一句）+ 字段级 `fieldNote`（`[tts].cache_entries` 这处"段内档位不一致"的例外）；**`live` 不标注**（默认预期，逐字段标注只会制造噪音） |
| 客户端 · 409 | `conflict` observable + 顶栏「重新加载」（**草稿保留**，与 §5.4-5 一致） |
| 新增组件 | `textNote(tone, micro)`（全站提示文字唯一入口）、`pluginStatusBadge(phase, enabled)`；`labeledField` 新增 `note` 参数 |
| 测试 | `config/tests/impact.rs`（7 条：字段级档位、段级回退、多段取最严格、无改动=live、旧写法迁移不算改动、数组元素变更可见、**每个配置段必须有档位**）；`host/tests.rs` 新增**管理页契约**测试（总览页读的每个键必须存在——客户端是字符串取键，改键名不会编译报错，只会让页面变空白） |

**验收（实跑）**：`cargo test` **206 passed / 0 failed / 3 ignored / 0 warning**（默认与 `--features sherpa` 同结果；⚠️ 这是 `cargo test` 的口径——`cargo check` 仍报 3 条**既有** dead_code，属预留 API，不在本批范围）；`./gradlew :apps:h5App:publishWeb` **BUILD SUCCESSFUL**，产物含 `总览`/`必需能力`/`已保存并立即生效`/`重新加载`/`未知配置页` 等全部新文案；`./gradlew :shared:compileKotlinMacosArm64` **BUILD SUCCESSFUL / 0 warning**。⚠️ 最后一步是必需的：`AdminShell`/`ConnectState` 在 `macosArm64Main`，`publishWeb` **不覆盖**它——只跑 web 会留下未验证的 macApp 路径。

**顺带清掉的历史噪音**：`aiui/tests.rs` 的未用导入 + 4 处 `data == null`（编译期恒假的死判断——`NMAllResponse` 的 `data` 是非空 `JSONObject`，这是照抄既有写法传染的）。清掉后两个 Kotlin 目标都 0 warning。

**与设计的偏差（5 条）**：

1. **统一动作模型仍未做**（`POST /api/plugins/{id}/actions/{name}` + job 轮询）：与 §0.10 偏差 8 是同一件事；UI 侧继续沿用 `probing` 轮询与测试台 `tts_test` 结果帧。
2. **§5.4-2「已覆盖 / 恢复默认」未做**：它要求服务端回答"这个字段是 `config.toml` 显式覆盖、还是回落到内置默认"，而 `GET /api/config` 返回的是**合并后的生效值**——要落地得先改服务端契约（区分"文件里有"与"内置默认"），属独立批次。
3. **`AdminTheme` token 未整体重写**：§5.8 的对齐表只被**新增**组件沿用（`textNote`/`pluginStatusBadge` 用既有 token）；既有组件的硬编码尺寸（44/48/64）未动——纯视觉回归风险，不该混在功能批次里。
4. **状态徽标文案硬编码在 `phase` 分支**（降级/异常/已关闭），没做成服务端 `PHASE_KEYS` 式映射表：这三个词不会随后端变化，且必须与 §5.3 的表格一致。
5. **总览页只有"清单 + 状态"，没有 §5.2 的二级页**（某能力 → 配置/状态/日志三页）：二级页需要"按能力取日志"的数据源，服务端目前没有该端点，硬做会变成假功能；当前用"总览行 → 对应配置 tab"的人工对应关系替代。

---

---

## 上下文管理界面与插件映射


### 6. 借鉴 DSH「插件理念」的具体落地

DSH（Cordis）插件体系实测要点（本机 `~/.dsh/profiles/desktop/`）：

- profile 根 `cordis.yml` 是**空数组**，注释明确「the tree is composed as patches: each bundle in package.json's `dsh.profile.bundles`, then `cordis.patch.yml`, then any `--patch` overlays. **Edit cordis.patch.yml, not this file**」。
- `cordis.patch.yml` 是**顶层 YAML 数组的 patch 条目**：`{id, name, config}`，支持 `!!js` 表达式（`dshHomePath(...)`、读 `process.env.*`）。
- 插件包自带 `dsh.bundle.patch`（`package.json` 的 `dsh` 字段）把自己**insert** 进树；插件作为普通 npm 依赖安装在 profile 的 `node_modules` 下，由 `.plugin-manager/logs/**/pnpm.log`、`.dsh-market/state.json` 记录。
- 插件代码只声明 `name` / `inject`（能力依赖）/ `apply(ctx, input)`（生命周期 + 配置入参），**不关心自己被谁装载、配置从哪来**。
- 宿主侧 GUI 支撑栈（**包名已逐个在 asar 中核对存在**）：`@deepseek-ai/dsh-host-plugin-inventory`、`dsh-client-ui-settings-plugins`（`settings.section` 壳 + `settings.plugins.tab` 列表槽）、`dsh-client-ui-plugin-manager`（可写侧栏 Plugins 页；数据源是 typert `@Remote` 的 `PluginManager.{listPlugins,listBundles,setPluginEnabled,setBundleEnabled,installBundle,removeBundle}`）、`dsh-client-ui-cordis`、`dsh-settings`、`dsh-config-editor`。
  ⚠️ **更正**：本文件早期版本写的 `dsh-client-schema-form` **不存在**（asar 内无此包），且 DSH 全仓检索 `configSchema` **0 命中**——「schema 自动渲染表单」在 DSH 里**没有落地实现**（`dsh-settings` README 原文：`autoGenerate`… *"no shipped client does so yet"*）。真实机制是：`dsh-settings` 把插件 `Config` **投影**成表单，且**只暴露 `.volatile()` 字段**（普通配置仍走 Cordis 配置文件），写入带 `expectedRevision`、过期写入被拒。

映射到本仓库（**不引入 Node/Cordis，只借机制**）：

| DSH 机制 | 本仓库对应改造 |
|---|---|
| 插件清单 + 插件 `Config`（**注意：不是"自动表单"**） | 每能力一个 `Config` 结构 + `Default`（现已有）；**新增 `GET /api/config/schema`** 返回 JSON Schema（`type/properties/default/description`）用于**校验 + 字段元数据 + 默认值**——前端 tab 仍手写（DSH 自己也没落地自动表单，见 §11.1） |
| bundle + patch 分层（默认→patch→overlay，`!!js` 读 env） | 配置分层沿用现状（内置默认 → `config.example.toml` → `config.toml`）；**补环境变量覆盖**（`XIAOZHI_MEMORY_EMBEDDING_API_KEY` 等），密钥可不落盘 |
| `inject = [...]` 能力声明 + 缺失即失败 | `Engines::new` 里按声明校验依赖（对齐 `runtime/mod.rs:64-71` 的 AIUI 三要素校验）；memory 声明需要 `llm`/`embedding`（embedding 可缺 → 自动降级 FTS5，须显式日志） |
| `apply(ctx, input)` 生命周期 | `plugins/mod.rs` 注册表 + `build_<capability>(&cfg)` 工厂（现有 `build_asr/build_tts/build_llm/build_memory`）；新增能力=加文件+注册一行 |
| 工具注册（`gm_*`） | 复用 `ToolSpec/ToolCall`（`llm/mod.rs:68-110`）；`gm_search` 首当其冲 |
| `systemPrompt` 服务改写 | 新增 `ContextAssembler`，替换 `openai.rs:130` 的静态 `instructions` |
| 热重载 / HMR | 复制 `engine_signature` 热切换范式（`runtime/mod.rs:139-166`）：`soul_sig` / `memory_sig` |
| 插件状态页（Settings→Plugins + inventory） | 管理页「记忆」tab：状态徽标 + 动作按钮；异步探测的轮询交互照抄 `voices.rs` 的 `probing` 模式（`agents.md` §5.2b） |
| **`enabled`（保存的选择）与激活态分开上报** | 「记忆」tab 必须能显示「已启用但**降级/加载失败**」（例：embedding 未配 → 已降为词法模式），否则用户只会得到"开了没用"的结论 |
| **表单写入带 revision，过期写入被拒** | `GET /api/config` 附带配置指纹（mtime+size 或内容 hash），`POST` 回传它，不一致返回 409 并提示重新加载——多标签页/多端同时管理是现实场景（现状是 last-write-wins：`ws/config.rs` 全无 revision 概念） |
| **密钥只存引用 + 表单打码**：config 里只有 `apiKeyEnv` 之类引用，值走独立 credential store；投影时 secret 字段只给 **presence 标记**，客户端未收到的字段在写入时**原样保留** | 🟡 **一半已落地（2026-10）**：「打码 + 保留」已完成——`SECRET_PATHS` + GET 摘除明文只回 `has_<field>`、POST「空串/缺失 = 保留原值、显式 `null` = 清除」、客户端 `putSecret` 不发送空密钥（见 `plugin-architecture-unification.md` §0.6）。**未做**的是 `apiKeyEnv` 引用式存储（密钥仍明文落在 `config.toml`）——那是 DSH 的 credential seam，需要时再加；届时新增 `[memory.embedding].api_key_env` 应优先于 `api_key` |

---

### 7. 管理界面设计

#### 7.1 Tab 布局（建议）

```
配置页 tabs:  Server | Audio | ASR | VAD | TTS | LLM | 灵魂 | 记忆
```

理由：LLM tab 现在只有 7 个字段（`ConfigCards.kt:287-301`）。若把人格（12+ 字段）与记忆（20+ 字段）塞进去，单页过长且语义混杂。**「灵魂」和「记忆」各占一个 tab，作为 LLM 的上游上下文独立配置**。

#### 7.2 「灵魂」tab

| UI 控件（官方组件） | 字段 | 默认值 |
|---|---|---|
| `labeledField` | name / address_user | 小智 / 你 |
| `labeledTextArea(h=90)` | self_intro | 空 |
| `labeledTextArea(h=120)` | worldview + backstory（或单字段 `backstory`） | 空 |
| `labeledTextArea(h=80)` | traits（每行一条「标签：行为描述」） | 空 |
| `labeledTextArea(h=80)` | values / boundaries | 空 |
| `labeledField` | tone / catchphrases | 空 |
| `labeledField`(数字) | max_sentences / max_chars | 2 / 40 |
| `switchRow` | emoji（语音默认关） | false |
| `labeledField`(大高度 `labeledTextArea`) | examples（Q/A 成对，2~4 组） | 空 |
| `labeledField` | profile_path（进阶：Markdown 档案） | ⛔ 未实现（§0.5 偏差 7） |
| `switchRow` | enabled（关闭 → 退回 `[llm].system_prompt` 老行为） | false |

#### 7.3 「记忆」tab

```
── 状态卡（只读，statusBadge/Text 官方组合） ──────────────
  后端: graph (db: /data/memory.db, 12.4 MB)   节点 342 · 边 891 · 待抽取 2 · 隔离 1
  向量: 关（本地词法：summary FTS5） / 开（model=text-embedding-v4, dim=1024）
  [ 刷新状态 ]  [ 立即维护 ]  [ 重建索引 ]  [ 导出 ]  [ 清空本设备记忆 ]

── 开关 ────────────────────────────────────────────────
  switchRow  enabled / recall_enabled / extraction_enabled

── 召回参数 ─────────────────────────────────────────────
  labeledField  fresh_turn_count / recall_max_nodes / recall_max_chars
  labeledField  semantic_score_threshold / recall_budget_ms
  switchRow     context_compaction_enabled（接管历史面）

── 抽取模型（独立路由，AlertDialog 下拉 = 复用 [llm] / 自定义） ──
  labeledField  extractor.api_base / extractor.api_key / extractor.model / extractor.max_tokens

── Embedding（AlertDialog 下拉：关 / OpenAI 兼容） ─────────
  labeledField  embedding.api_base / embedding.model / embedding.dimensions
  labeledField  embedding.api_key（或 api_key_env → 只写环境变量名）
  [ 测试 Embedding ]  → POST /api/memory/embed_test → 显示维度与耗时

── 记忆测试（**最重要的可用性反馈**） ─────────────────────
  labeledField  输入一句话
  [ 测试召回 ]  → POST /api/memory/recall → 显示：命中节点数 / 耗时 / 注入正文预览
                 并醒目提示「本内容将作为背景资料注入 system prompt」

── 保留策略 ────────────────────────────────────────────
  AlertDialog 下拉 keep(all|referenced) + labeledField recent_turns / retention_days
  switchRow    dry_run（只演练不删除）
```

#### 7.4 交互与生效语义

- 保存后 Toast 文案沿用现有约定：「配置已保存到 <path>；引擎相关参数需重启 server 生效。」→ 记忆/灵魂支持**热切换**，文案可改为「…；新会话生效」（对齐 TTS 热切换的既有语义）。
- **AIUI 全链路模式必须显式提示**：`[aiui].enabled=true` 时流水线走云端闭环，本地 LLM 不参与（`config.rs:46-50`、`runtime/mod.rs:65-71`），灵魂/记忆**不生效**。UI 上在「记忆」tab 顶部给红字提示 + `vif({ form.aiuiEnabled })`。
- 表单状态：`ConfigFormState` 增加对应 observable（`ConfigFormState.kt:118-125` 的现有风格）+ `buildConfigJson`（`:435-444`）+ `applyConfig`（`:531-538`）双向映射；服务端往返单测（`ws/config.rs:233`）必须扩展覆盖新段。

---

---

## 健康和记忆前端验收


### 6. 前端触点（管理台位置明细）

| 触点 | 位置 | 实现要点 |
|---|---|---|
| 就绪度横幅（P1-6 P0） | 概览页最顶部（实时卡之上），AdminPage `overview` section | `healthBanner`：仅 deviation 行渲染，可点击跳配置页对应 section；AdminNav 不加项 |
| 能力健康卡（P1-6 P1） | 配置页「总览」section（`ConfigSections` 的 `overview` 置顶项，order 0）内、既有 `overviewCard`（能力总览）之下 | `engineHealthCard`：11 行 × phase；deviation 打标 + 原因 + 可点击；正常行弱化 |
| AdminNav 常量表 | **本轮不加导航项**（P1-6/P1-7 均挂在既有 overview/config section 内，无新页面） | — |
| AdminPage 接线三处 | ① `val engineHealth = EngineHealthState(this)`；② `pageDidAppear` 加 `engineHealth.load(this)`；③ `renderOpsOverview` 调用点传 health + 跳转 lambda | 比照 metrics/usage 既有模式 |
| 统一壳（macOS） | 已废除原 AdminShell @Page("router") 双壳：macApp 改开 `@Page("admin")`，测试台/切换服务器/基址经 shared AdminPlatform expect/actual 挂进统一壳 | 历史记录：该壳曾独立服务「测试台/配置」两个 section |
| ConfigPage（独立页） | 同 AdminPage 三处 | `?page_name=config` 单页模式同样生效 |
| 记忆 engine 下拉（P1-7 P1） | 配置页「记忆」section（ConfigSections `memory`，order 130）的 engine 下拉 | `memoryBackendOptions` 加 store 项；选中 store 时 textNote 定位说明（"只存原文不抽取，适合先积累再决定"） |
| `/api/memory/status` 面板数据 | 配置页记忆卡的状态区（ConfigContextState.memoryStatusMsg 链路） | engine 变 "store"、新增 turns 计数——字段 additive，前端按需展示 |

---

### 7. 待明确事项

1. **T2-5 记忆记录管理端点是否本轮实施**：PRD 需求池未列（P0 止于注册表/store/数据不丢），团队负责人任务书提及"列表/查单条/删单条"故设计为可选任务（独立查询、不进 trait）。**需拍板：做 / 推迟 P2**。本设计推荐：推迟——`gm_messages` 已有 (scope, turn_index) 索引，成本确实低，但管理 UI 不在 PRD 前端触点表内，只做端点没有消费方。
2. **`Capability::Memory` display 文案**：从「记忆（图记忆）」改「记忆」是 UI 可见文案变化（多后端后原文案不准确）。若无异议随 T2-3 生效。
3. **store 后端的 maintain 语义**：本设计让保留策略（retention）对 store 数据生效（复用 graph 的 retention 代码，零新逻辑）；若期望"纯存储=存了永不删"，需在 StoreOnlyMemory 里跳过 retention——**默认前者**（用户可用 `[memory.retention]` 显式控制），需确认。
4. **memory 目录可写探测的微副作用**：探测会创建并立即删除 `.health-probe` 临时文件（磁盘写、非网络非引擎构建）。PRD 验收 6 字面是"不产生引擎构建或网络请求"，本设计合规；若要求严格零磁盘写，可降级为仅 `metadata` 检查（代价：区分不了"目录存在但只读"）。
5. **store 积累数据的回填入口**：本设计明确**不自动回填**（切回 graph 不抽取 'stored' 行，防成本惊吓）；"批量重建图记忆"入口（PRD P2 导出同批）留待 P1-9 轮后与用户确认交互形态。
6. **PRD P1-6 的 P2 项**（自检历史快照、保存前草稿自检）本轮不做，设计未包含——如需提前请明示。

---

### 8. 验收对照（PRD → 设计映射）

#### P1-6（PRD §3.2 验收标准）

| # | 验收标准 | 设计覆盖 |
|---|---|---|
| 1 | `GET /api/health/engines` 返回 11 条记录、字段齐全 | T1-1：ALL_CAPABILITIES 遍历 + capability_summary + probe；响应 schema §2.1（capability/display/enabled/phase/last_error 齐全） |
| 2 | `memory.enabled=false` → phase=disabled、last_error 空 | disabled 项不跑 probe、last_error=None（plan 语义既有：未选中 → (Disabled, None)，host.rs:122-124） |
| 3 | db_path 不可写启动 → memory 行 degraded 含修复指引，语音不受影响 | 启动降级 = `build_optional` 既有行为（host.rs:284-297 mark Degraded）→ host.snapshot 透出；端点侧 memory 目录探测 Fail → Degraded（可选能力），文案含怎么修；语音不受影响 = 可选能力分级既有契约 |
| 4 | 必需能力缺模型 → failed（以 plan 单测覆盖） | probe 文件存在性 Fail + required → Failed 合并规则；单测在 plugins/health/tests.rs（无 sherpa feature 可跑：用临时路径模拟缺失） |
| 5 | 概览页横幅「记忆 · 降级」可点击跳配置记忆 section | T1-2 healthBanner + config_section 映射 + form.configSection 跳转链路 |
| 6 | 无副作用：连调两次一致、零构建零网络 | probe 仅本地 fs；handler 纯读 host snapshot + live_config；幂等单测（护栏 7） |

#### P1-7（PRD §3.2/§3.3 验收标准）

| # | 验收标准 | 设计覆盖 |
|---|---|---|
| 1 | engine=store：N 轮 → N 条原文轮次；摘要/倒排零写入；token 零增量 | StoreOnlyMemory.record → insert_turn_state('stored')（gm_messages 唯一写路径）；代码无引用 gm_turn_memories/gm_summary_terms 的路径；extract_pending 恒默认值（零 LLM）；构造不建 LLM 客户端；护栏测试 3 |
| 2 | engine=store：recall 恒空、上下文无记忆块（原文仍记录） | StoreOnlyMemory.recall 恒 RecallResult::none()（绝不 Err）；与 enabled=false 的差异仅在 record 落库 |
| 3 | graph→store→graph 数据不丢、旧记忆可召回 | 同一 db_path；schema::open 迁移幂等；store 只写 gm_messages（'stored'），graph 表零触碰；护栏测试 4 |
| 4 | engine=store2 → 配置加载报错、含怎么修与合法值 | 自定义 Deserialize 中文报错（§1.2.3），Config::load 与 PUT /api/config 两条路径统一拦截；护栏测试 2 |
| 5 | 默认回归：不写 engine → graph；enabled=false → NoopMemory 零成本 | 枚举 Default 不变；active() 扩展对 enabled=false 恒 false；既有测试全绿 + 护栏测试 5 |
| 6 | 管理台下拉三选项；保存 store 后 status 后端标识变 store | memoryBackendOptions 加项（T2-4）；`/api/memory/status` 的 engine 来自 `stats().engine` = "store"（自动）；下拉 schema 半驱动的现状澄清见 §1.2.7 |

#### 全局非功能

| # | 约束 | 覆盖 |
|---|---|---|
| NF-1 | 离线单二进制 | 零新增依赖（§4）；新存储复用既有 SQLite |
| NF-2 | TOML 落盘语义不破坏 | 本轮不动 `Config::load`/`put_config`/SECRET_PATHS（那是 P1-5）；`alias="backend"` 旧键不受自定义 Deserialize 影响 |
| NF-3 | 中文注释与怎么修 | probe 文案 / Deserialize 报错 / store 定位说明均按口径；新模块 `//!` 中文文档注释 |
| NF-4 | 零成本默认 | 默认配置行为与二进制依赖零变化；护栏测试 5 |
| NF-5 | 只读投影 | 健康端点纯投影（§2.1 幂等性）；probe 唯一微副作用见待明确 #4 |
