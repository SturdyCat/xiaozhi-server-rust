# 架构设计：NLU 技能概念统一与按控制域分组

> **归档说明（归档波次：W15-2，旧端软下线）**：本文为 gearui-kit/echomuse/tuya 计划与验收/架构 原文，文中「Kuikly / GearUI / gearuiKit / client/shared / publishWeb / h5App/web / sharedCompose」等均为当时旧端口径，**保留史实、不作现行实现依据**。当前管理台为 `client/admin-web`（React + Vite + shadcn/ui + Tauri 2，生产静态目录 `client/admin-web/dist`）；见 [admin-web 指引](<../../client/admin-web/AGENTS.md>) 与 [W15 资产盘点归档](<./admin-web-migration/w15-legacy-assets-research.md>)。

- 对应 PRD：`docs/plans/nlu-skill-domain-prd.md`；架构师：高见远；日期：2026-02
- 铁律复核（已对照代码确认）：`end_session`/`other` 保留标签（catalog.rs:95）；MCP 工具名静态字面量（plan.rs:174）；训练标签=技能名；ESP 协议不动。

## 1. 目标概念模型与数据结构变更

### 1.1 domain 字段落点：IntentDef 加 `Domain` 枚举字段——够

```rust
// plugins/nlu/mod.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Domain { #[default] Server, Device, Iot }

pub struct IntentDef {
    pub name: String,
    #[serde(default)] pub templates: Vec<String>,
    #[serde(default)] pub action: ActionDef,
    #[serde(default)] pub domain: Domain,   // 新增
}
```

为什么这样就够：
- IntentDef **没有** `deny_unknown_fields`，旧保存态 JSON / 旧 manifest 的 `Artifact.nlu` 反序列化天然通过；
- enum 即白名单（device/server/iot），无需额外字符串校验代码，非法值直接反序列化失败并进入既有"保存态无效"链路；
- `iot` 不拒绝：P2-1 数据模型预留，仅影响分组展示。

### 1.2 缺省规则（旧配置/旧 manifest 兼容）

| 场景 | 缺 domain 时 | 依据 |
|---|---|---|
| 旧保存态 nlu intents | `Server` | 现网保存态只有 reminder_*（server），与事实一致 |
| 旧 manifest 的 `Artifact.nlu` 内 intents | `Server` | 仅历史展示；执行按标签名，不受影响 |
| effective_catalog 注入的 end_session | **显式 `Domain::Device`** | catalog.rs 注入处硬编码，见下 |

`catalog_json`（skill_api.rs:114-124）的 `default` 判定用完整序列化比较：旧保存态 intent 读入后序列化会带上 `"domain":"server"`，与新 defaults.json 的种子值相等 → default/custom 判定不变；`end_session` 走名字判定 → 仍为 command。

### 1.3 end_session 的 domain 与 command 词表去留（关键取舍）

**决策：保留 `[command]` 配置结构与 effective_catalog 注入逻辑，仅重定义语义**——command 词表 = 设备域技能 `end_session` 的训练样本来源，注入时 `domain: Domain::Device`。

理由：
1. **硬约束**：`end_session` 禁止出现在 nlu intents（catalog.rs:95），把词表挪进 nlu 种子会破坏该不变量，牵连 validate、训练、管理台保存链路（保存 end_session 普通技能会直接校验失败）；
2. **同源链路已锁定**：manifest/catalog_source/测试均以"effective_catalog(command, nlu)"为口径，保留注入点改动面最小；
3. **词表增删链路独立可用**：ConfigCommandCard 保存的是 config.command，与 nlu intents 保存互不干扰（回答 Q6）。

## 2. 模块影响清单（逐文件）

### 服务端（server/）

| 文件 | 改动 |
|---|---|
| `plugins/nlu/mod.rs` | 新增 `Domain` 枚举；`IntentDef` 加 `domain` 字段；文档注释更新（"意图"→"技能"） |
| `plugins/nlu/defaults.json` | 11 个 reminder_* 加 `"domain":"server"`；新增 2-3 个设备域示例技能（见 §4） |
| `plugins/skill/catalog.rs` | effective_catalog 注入 end_session 时 `domain: Domain::Device`；validate 无需新校验（enum 即白名单）；模块注释重定义 command 语义 |
| `plugins/command/mod.rs` | 仅注释与错误文案改口（"end_session 训练词表"，归设备域）；结构不动 |
| `src/gateway/ws/skill_api.rs` | `catalog_json` 的 `source_semantics` 说明更新（提到 domain 与 end_session=设备域）；intents 已全量序列化，domain 自动透出，无需结构改动 |
| `src/gateway/ws/nlu_build.rs` | `catalog_source` 升级为 `"effective_command_nlu_v2"`（L278） |
| `src/gateway/ws/nlu_build/tests.rs`、`artifact_tests.rs`、`predict_tests.rs`、`src/gateway/ws/nlu_model/tests.rs` | fixture/断言同步（v2 字符串、新 IntentDef 构造补 domain） |
| `plugins/nlu/tests.rs`、skill 目录相关测试 | 补 domain 序列化/缺省/注入用例 |

### 管理台（client/shared/.../admin/）

| 文件 | 改动 |
|---|---|
| `ConfigNluCatalog.kt` | 平铺列表 → 按 domain 分组（设备/服务器/IoT 三段）+ 域筛选 + 技能名搜索；end_session 显示为设备域"断开对话"；IoT 组渲染"规划中"占位文案（P2-1 展示部分） |
| `ConfigNluCard.kt` / `ConfigNluState.kt` / `ConfigFormState.kt` | 编辑草稿加 domain 选择（下拉/分段，默认 server，P1-2）；保存改名技能时强提示"训练标签变更，旧模型失效，需重新训练"（P0-4，沿用保存→训练链路，只加提示不拦截既有流程） |
| `ConfigCommandCard.kt` | 文案改为"断开对话（end_session）技能训练例句"，注明归设备域；编辑能力保留 |

## 3. 兼容与迁移策略

1. **保存态配置**：无 domain 字段 → 缺省 server，**不迁移、不改写**已保存 JSON（延续"不偷偷改保存态"的项目铁律）；管理员可在管理台逐个改域（仅影响展示）。
2. **旧 manifest 与旧模型**：旧 manifest 的 `catalog_source: effective_command_nlu_v1` 保留可读（Option 字段同一兼容范式）；旧模型 bert_label_map 按技能名映射不变——本次**不改任何默认技能名**（end_session 是注入名非 intent 名），故旧模型标签不受影响，无需强制重训。
3. **catalog_source 升 v2**：必要性=审计口径（effective catalog 的 intent 结构带了 domain 字段）；训练输入 input.json 只含 name/templates（nlu_build.rs:250），**domain 不进训练**，trainer 脚本零改动；artifact_tests 的 None/Some 范式测试不受影响，仅更新 v1→v2 字面量断言。
4. **ESP 固件协议不受影响的论证**：本次改动全部位于服务端保存态结构（config JSON）与管理台；语音链路（hello→discover→tts/mcp 帧）、MCP 工具发现（discovery.rs）、模型产物三件套均不触碰；管理台新读的 `intents[].domain` 来自服务端 JSON 新增键，旧服务端不给该键时客户端 `optString("domain")` 缺省处理，前后端各自灰度安全。

## 4. 设备域示例技能（可直接入库 defaults.json）

**动作选型：单工具直调用 `"mcp:<tool>"`，不用 plan→devices.call**。理由：槽位自动映射为 arguments（command.rs:150-155）；未声明工具走既有四降级路径（Unsupported/missing/unknown-tool/transport，command.rs:102-114 注释）给固定话术不崩会话，无需新机制；plan 只留给多步骤场景。

```json
{"name":"device_volume_set","domain":"device",
 "templates":["音量调到{volume}","把音量设为{volume}","把音量调成{volume}"],
 "action":{"kind":"mcp:self.audio_speaker.set_volume","reply":""}}
{"name":"device_mute","domain":"device",
 "templates":["静音","把设备静音","关声音"],
 "action":{"kind":"mcp:self.audio_speaker.set_mute","reply":""}}
{"name":"device_pause_playback","domain":"device",
 "templates":["暂停播放","暂停音乐","先停一下"],
 "action":{"kind":"mcp:self.audio_player.pause","reply":""}}
```

注意：槽位名 = 固件工具参数名（`volume`）；工具名以各板型 discovery 上报为准，非目标板型执行时降级话术拒识（回答 Q5）；mcp 槽位 arguments 走字符串值，音量数值类参数由固件侧解析。

## 5. 任务列表（按实现顺序）

| ID | 任务 | 文件 | 依赖 | 验收 |
|---|---|---|---|---|
| T01 | 服务端数据模型：Domain 枚举 + IntentDef.domain + end_session 注入 device | `nlu/mod.rs`、`skill/catalog.rs`、`command/mod.rs`、`nlu/tests.rs`、skill 目录测试 | — | `cargo test plugins::nlu`、`cargo test skills::`；旧保存态 JSON 样例反序列化通过 |
| T02 | 种子重组 + 示例技能 + catalog_source v2 | `nlu/defaults.json`、`nlu_build.rs`、`nlu_build/tests.rs`、`artifact_tests.rs`、`predict_tests.rs`、`nlu_model/tests.rs` | T01 | `cargo test gateway::ws::nlu_build`、`cargo test gateway::ws::nlu_model`；defaults.json 与 manifest 内 catalog_source=v2 一致 |
| T03 | 目录 API 语义透出 | `gateway/ws/skill_api.rs`（source_semantics、end_session 来源说明）+ skill_api/tests.rs | T02 | `cargo test skill_api`；/api/skills/catalog 返回 intents[].domain |
| T04 | 管理台：分组展示/搜索/域筛选/IoT 占位 + domain 编辑 + 改名强提示 + CommandCard 文案 | `ConfigNluCatalog.kt`、`ConfigNluCard.kt`、`ConfigNluState.kt`、`ConfigFormState.kt`、`ConfigCommandCard.kt` | T03 | `./gradlew :shared:compileKotlinMacosArm64`；文件 ≤600 行；GearUI stateVar/stateListOf 规范 |
| T05 | 统一目录文档（P1-3）：effective_catalog 说明 + 各默认技能域/动作/工具依赖表 | `docs/`（新增 skill-catalog 说明文档） | T04 | cargo test 全量回归（skills::、nlu、nlu_build、skill_api）通过 |

依赖图为线性 T01→T02→T03→T04→T05（数据模型是种子与 API 的前提，管理台消费 API 字段）。

## 6. 待明确事项（PRD Q1-Q6）与推荐答案

| # | 推荐 | 理由/影响 |
|---|---|---|
| Q1 | **按执行者分域**（默认方案） | 与动作 kind 天然对齐（mcp:→device，plan→server），无歧义规则可编程；用户感知差异用分组命名（"设备控制/服务器服务"）弥补。若改按感知分域，device_volume_set 等示例需整体改归 server，示例技能与目录文案返工 |
| Q2 | 重启归设备域，走固件 MCP 工具；本次不做服务端设备管理动作 | 无重启工具的板型由降级话术拒识即可；服务端重启通道是新能力，超出本次范围 |
| Q3 | 是，iot 仅占位 + 模型预留 | Domain::Iot 已进枚举，管理台显示"规划中"，不做任何连接实现 |
| Q4 | 不做旧名别名（P2-2 延后），允许直接改名（需重训） | 别名会破坏"训练标签=技能名"与目录同源链路的简单性；审计用 catalog_source 版本 + build_id 已够 |
| Q5 | 复用既有四降级路径固定话术，不改协议 | command.rs 现成机制满足"不报错崩会话"，零新代码；示例技能 reply 留空即由该路径出话术 |
| Q6 | 词表保留独立文件（command.keywords），管理台仍编辑 CommandCard；不做普通技能例句编辑 | end_session 禁入 nlu intents 的硬约束决定其保存链路必须独立；合并进技能例句编辑会引入保留名特判，得不偿失 |
