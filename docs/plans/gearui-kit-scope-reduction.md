# GearUI 迁移 · 范围削减决定（2026-10-29）

> 状态：**已生效**。本文件是 T10–T13 的范围权威，与 `gearui-kit-migration-architecture.md` §5 冲突时以本文为准。
> 依据：用户指令「项目主要是配置与测试，以及设备管理与调试，其他不相关的都可以移除简化项目，方便适配 gearui-kit」；
> 并经确认：文件管理保留（属配置相关）、EchoMuse 六 tab 全保留（Q6 维持）。

## 一、范围矩阵（12 个一级导航 → 8 个）

| 导航 id | 处置 | 现状 |
|---|---|---|
| `overview` | **保留** | T09 已迁移（OpsOverviewPage.kt） |
| `devices` | **保留** | T09 已迁移（OpsDevicesPage.kt） |
| `sessions` | **砍** | T09 已迁移 → 本轮删除 |
| `conversations` | **砍** | T09 已迁移 → 本轮删除 |
| `usage` | **砍** | T09 已迁移 → 本轮删除 |
| `metrics` | **砍** | T09 已迁移 → 本轮删除 |
| `config` | **保留** | T07 已迁移 |
| `settings` | **保留** | T08 已迁移 |
| `echomuse` | **保留（六 tab 全留）** | T10 迁移中（5/12 文件已转 Compose） |
| `console` | **保留** | T09 已迁移（ConsolePage.kt，深色终端 K2） |
| `files` | **保留** | T09 已迁移（FilesPage.kt）——用户裁定属配置相关 |
| `testbench` | **保留** | 待 T12 重写（macOS 专属） |

## 二、砍除清单（工程步骤，按序执行，每步保编译）

对象：`client/shared/src/commonMain/kotlin/com/xiaozhi/admin/`（下称 `admin/`）

1. **删 4 个页面文件**：`OpsSessionsPage.kt`(185) / `OpsConversationsPage.kt`(101) / `OpsUsagePage.kt`(142) / `OpsMetricsPage.kt`(150)。
2. **`AdminShell.kt`**：删 `sessions` / `conversations` / `usage` / `metrics` 4 个分支（现 300–326 行区域内）。**不动** `overview` / `devices` / `console` / `files` / `config` / `settings` 分支。
3. **`contracts/AdminNav.kt`**：删上述 4 个 `NavDescriptor` 项（12 → 8）。`init` 里的 id 唯一 / order 升序 / 非空 / DEFAULT 存在 / 同组相邻断言必须仍通过。
4. **`AdminPage.kt`**：删 `page.sessions` / `page.conversations` / `page.usage` / `page.metrics` 对应 state 字段声明、`XxxState(this)` 构造、`refreshAll` / loader 里的对应请求。sessions 数据若出自 `OpsState`，只删字段不删类（`OpsState` 同时服务 overview/devices）。
5. **删 3 个 state 类**：`OpsConversationsState.kt`(127) / `OpsUsageState.kt`(90) / `OpsMetricsState.kt`(87)（先 grep 确认无其他引用）。`OpsState.kt` 保留，仅删 sessions 相关字段（若有）。
6. **`OpsScaffold.kt`**：清理只被已砍页面使用的零件；`OpsPageFrame` / 骨架屏 / 空态 / 磁贴等被 overview/devices/files/console 共用的**保留**。
7. **`adminCoreTests` 的 `UiContractsTest`**：导航断言随 8 项更新；`everySectionBelongsToExactlyOneMenu` 必须继续绿。
8. **同步护栏常量**：`verifyComposeStateMigration` 的 `expectedStateVarCalls` / `expectedStateListCalls` 会因删 state 类而下降——**用 codemod `--check` 的真实输出回填新常量**（当前 482/51，删除后以实测为准），并在 build 脚本注释里写明「T-削减 批次删除 3 个 state 类所致」。禁止为凑旧数字保留死代码。
9. 静态断言（同 T09 口径，全仓 `-E` 正则）：`vfor` 0、自研 token 0（注释行除外）、每文件 ≤600。

## 三、T10–T13 执行序列（限流恢复后）

| 批次 | 内容 | 执行者 |
|---|---|---|
| **R1（削减）** | 上述砍除清单 1–9 | 工程师 B |
| **R2（T10 收尾）** | EchoMuse 剩余 7 文件迁 Compose（六 tab 全保留，原约束不变） | 工程师 A |
| R3（T11 验收） | `OtaCard.kt` 已重写为 478 行 Compose（10 个 `@Composable`），补跑六件套验收 + 上传链路确认（picker actual 复用、`ConfirmDialog(destructive)`、进度反馈） | 工程师 B |
| R4（T12） | 测试台重写（`TestBenchPages.kt` 648 需拆、`TestBenchNluPage.kt` 139、`AdminPlatform.kt` actual 478；`benchConfigHandoff` 换回 `ConfigSectionHost`）＋ 清理清单六步（`FieldHelpHost` 迁 contracts → 删冗余 `import ...ui.*` → 删 `compose/` → 删 `:sharedCompose` → 删 `ui/` 与三个 Legacy 桥 → 护栏 `bridgeWhitelist` 清空且 legacy=0） | 两工程师分工待定 |
| R5（T13） | 四端编译 + 全仓断言（`grep -rlE "vfor\(|ViewContainer<"` 仅允许 `gearuiKit` 上游；自研 token 断言需排除注释行：`grep -rnE "AdminColors\|..." \| grep -vE ":[[:space:]]*(\*|//)"`）+ `publishWeb` + 交互清单交用户本地验证 | 主理人主导 |

## 四、不变式（仍然有效）

- 契约层字段与签名零改动（砍除只删**整页**，不改保留页的状态结构）。
- 提交路径唯一：一次 `POST /api/config` + 设备 overlay 差量。
- 控制台 / EchoMuse 控制台 tab 保持深色（K2）；删除类操作一律 `ConfirmDialog(destructive)`（K4）。
- 六 tab 全保留（Q6）；图标 26 个一对一（K7）；断点 768/900/700/240 一字不改（K9）。
- 不 `git add` / `commit` / `checkout`；失败即停、如实标注「未执行/需本地验证」。
