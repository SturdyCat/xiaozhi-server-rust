# W13 / Settings 页前置只读研究 · 权威归档

> **归档说明（归档波次：W15-2）**：本文为 W12/W13/W14b 阶段**研究原文归档**，记录当时旧端权威与拍板依据，保留史实；当前实施口径见 [W15 资产盘点](<./w15-legacy-assets-research.md>) 与 [W15 构建链](<./w15-production-build-research.md>)，管理台当前为 `client/admin-web`（见 [admin-web 指引](<../../../client/admin-web/AGENTS.md>)）。

> **文档性质**：W13 / Settings 页迁移的**唯一权威契约**，供工程师实施与 QA 回归逐节核对。由研究阶段发送给 team-lead 的「报告 1/2 + 报告 2/2」**原文全量合并**而成，研究原文一字未删。
> **研究纪律**：研究全程只读——未创建/修改任何源码文件、未执行构建、未做 git 操作；本归档是经 team-lead 授权写入的单一文档。研究期间工程师并行修改 `client/admin-web/package.json` / `.prettierrc` / `AGENTS.md`，研究结论未把这些文件的状态变化当作事实（占位页内容除外）；1/2 交叉印证的 `AGENTS.md` 是 `client/shared/.../admin/AGENTS.md`（旧端侧），不在被改范围。
> **证据口径**：全部结论带 `file:line`；逐字文案给中文原文。根路径 `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/`；三段路径基准见各报告开头。
> **口径修正（本归档以代码为准）**：此前派发口径「复用 Config **8 全局段**」为**过时口径**，真实为 **6 段**。`ConfigSections.global`（`client/shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigSections.kt:66`）按 `PipelineFlow.primaryForSection(it.id) == null` 动态判定；三重口径链为 8（任务口径）→ 7（`ConfigSections.kt:60-61` 旧注释，含 echomuse/mcp、不含 tuya）→ 6（代码 + `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md:49/:71`）。`echomuse.input`（`PipelineFlow.kt:63-65`）与 `mcp.tools`（`:87-89`）加入流程节点后移出全局，新增 `tuya`（`ConfigSections.kt:47`）移入，净 7→6。**实施与 QA 一律以 6 段为准：overview / presets / aiui / server / tuya / ota。**
> **形态结论（先看）**：旧端 settings 不是独立页面、不是纯跳转、不是「几个开关」，而是侧边栏一级菜单「全局配置」——Config 工作区的第二个视图，与「语音流程」共用同一份 `ConfigFormState`、同一套 `ConfigSectionHost` 卡片、同一「保存配置」按钮、一次 `POST /api/config`；服务端**无 Settings 独立端点**；admin-web 6 段 **100% 已实现**于 ConfigPage，需新增 0 字段 / 0 端点 / 0 api 封装。

## 已拍板结论（用户经 team-lead 确认，2026-11-11）

| 编号 | 拍板项 | 结论 |
|---|---|---|
| A（形态） | Settings 页形态 | **方案 A 过滤视图**：Settings = `ConfigPage` 全局段过滤视图，同一 `ConfigDraftProvider` / `SaveBar` / 6 个 Section 组件（overview / presets / aiui / server / tuya / ota），仅左菜单按 `ConfigSections.globalGroups` 只列这 6 段、共用顶部「保存配置」。不新建第 3 份表单 / 第 3 套 draft。 |
| nav | nav 描述符对齐旧端 | `label`「系统设置」→ **「全局配置」**；`description`「管理员账号、通知与服务运行参数。」→ 改为旧端真实语义（建议逐字取 `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md:71`）。`SettingsPage.tsx` 占位页「暗色主题切换」「Kuikly adminCore · 设置模块」等臆想文案一并修正；`NAV_DESCRIPTORS` 保持 10 项，settings **不新增为第 11 项**。 |

> 以上为**实施依据**。下方「报告 1/2」「报告 2/2」均为**研究原文全量保留、零删减**：2/2 §9 的「A/B 二选一、必须先拍板」是研究阶段的待拍板表述，用户已拍板 **A**；方案 B 及其相关表述作为研究原文保留，实施按上表裁减即可。原文 §9「保持 Tuya 凭证 3 项不渲染（C6）、`OtaSection` 0 配置字段、共用 draft、一次 `POST /api/config`（部分更新 + `expected_revision`）」为**拍板后仍须遵守的回归检查点**。如遇口径冲突，以原文逐行核读的源码 `file:line` 为准。

## 目录与合并顺序

| 区段 | 内容 | 来源 |
|---|---|---|
| 报告 1/2 原文（§0–§5） | 形态结论与路径基准、导航/路由、「8 全局段 → 真实 6 段」结构性证伪链、6 段逐字盘点、State 与请求纪律逐字文案、服务端权威（`/api/config` 系列无鉴权 / GET 密钥打码 / PUT 部分更新 / 409 文案 / meta / presets 4 端点 / schema） | 报告 1/2 原文 |
| 报告 2/2 原文（§6–§11） | admin-web 现状（nav 第 5 项占位 + 占位页 + tauriOnly 过滤）、6 段 100% 重叠排查表、api 封装检查单、实施清单与拍板项、11 条诚实化文案锚点（全部带 file:line）、三句话总结 | 报告 2/2 原文 |

---

---

> **以下为「报告 1/2」原文，全量保留，未删减。**

---

W13 / Settings 页前置研究 **报告 1/2（旧端 Kotlin + 服务端权威）**

路径基准（下文 `文件:行` 均相对此，便于追溯绝对路径）：
- 旧端 Kotlin = `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/client/shared/src/commonMain/kotlin/com/xiaozhi/admin/`
- 服务端 = `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/server/src/`
- admin-web = `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/client/admin-web/src/`

## 0. 一句话形态结论（先给答案，不虚扩）
旧端 settings **不是独立页面、不是纯跳转、不是「几个开关」、不是即时生效型**。它是侧边栏一级菜单「全局配置」(`contracts/AdminNav.kt:58` `NavDescriptor("settings", 55, "全局配置", GROUP_CONFIG)`)，本质是 **Config 工作区的第二个视图**：与「语音流程」共用同一份 `ConfigFormState`、同一套 `ConfigSectionHost` 卡片、同一顶部「保存配置」按钮、一次 `POST /api/config`。**0 个 Settings 专属配置字段、0 个 Settings 专属端点、0 个独立 save**。若 admin-web 要「迁移 Settings」，真实工作量是「给已存在的 6 个全局配置段换一个入口视图」，不是新建一组表单。（nav 第 11 项之问见 2/2：不是第 11 项，已是第 5 项占位。）

## 1. 导航与路由：settings 与 config 是同一工作区的两个视图
- 一级菜单 `contracts/AdminNav.kt:58`：label 是「全局配置」，**不是「设置」**。
- 共用状态 `AdminPage.kt:71-72`：只构造一份 `ConfigFormState`，同时喂给两个页面槽位。
- 路由分发 `AdminPage.kt:148-155` `selectSection`：`config → openWorkflowPage()` / `settings → openGlobalPage()`；`:161-163` `onSectionRouted` 用 `ConfigSections.isGlobal(section)` 反推落哪个菜单。
- 双路由记忆 `ConfigFormState.kt:327-328` `lastWorkflowSelection` / `lastGlobalSection`；`:334-344` `openWorkflowPage/openGlobalPage`；`:366-369` `selectGlobalSection`。
- 权威注释逐字 `ConfigWorkspaceViews.kt:224-230`：「「全局配置」页布局（侧边栏独立菜单）：**没有**语音工作流轨道、也**没有**配置/测试 Tab。」「左侧是全局 section 的分组菜单（总览 / 上下文增强 / 服务与接入），右侧复用 [ConfigSectionHost] 渲染同一套卡片；保存仍由顶部「保存配置」按钮统一提交（与「语音流程」共用一份 [ConfigFormState]、一次 `POST /api/config`）。」
- 菜单渲染 `ConfigWorkspaceViews.kt:256-286`：遍历 `ConfigSections.globalGroups`，`WorkspaceRow(label = section.label, …)`(:277)；分组名 `ConfigSections.kt:74` `listOf("总览") + groups.map{it.second}` →「总览」「语音主流程」「上下文增强」「服务与接入」(:23-26)。
- 单页标题 `ConfigPage.kt:273-277`：流程节点=「节点管理」，全局 section=「全局配置」。
- 保存控件 `ConfigPage.kt:247-263` 三态逐字：`:249`「保存中…」/`:250`「保存配置」/`:251`「保存设备配置」；`:241`「重新加载」；`:254-256` 注释「toast 在**接口返回后**由 save 回调触发（不能在 save() 之后立即取 statusMsg：那时请求还没回来，会弹出上一条旧消息）」。

## 2. 「复用 Config 8 全局段」口径证伪（核心：真实 6 段，三种口径互相打架）
结论：**「8 全局段」是过时口径，代码真实为 6 段**，不要写进实施规格。
- `ConfigSections.global`(`ConfigSections.kt:66`) **不是硬编码清单**，是动态判定 `all.filter { PipelineFlow.primaryForSection(it.id) == null }`。
- 过时注释 `ConfigSections.kt:55-65`（:60-61 逐字）：「其余（能力总览 / 预设 / AIUI 全链路 / 服务 / **EchoMuse 接入 / MCP** / OTA）在流程里没有位置，集中到侧边栏独立菜单「全局配置」」= **7 段**，含 echomuse/mcp、**不含 tuya**。
- 判据 `PipelineFlow.kt:123-126` `primaryForSection`：audio 特判 `forId("output")`、memory 特判 `forId("memory.writeback")`，其余 `all.firstOrNull{ it.id==sectionId && it.sectionId==sectionId } ?: all.firstOrNull{ it.sectionId==sectionId }`；`FlowFeatureDescriptor` 第 5 个位置参数是 `sectionId`(`PipelineFlow.kt:25`)。
- 后来加入的节点使 2 段移出全局：`PipelineFlow.kt:63-65` `echomuse.input`(sectionId="echomuse")、`:87-89` `mcp.tools`(sectionId="mcp")；同时 `ConfigSections.kt:47` 新增 `tuya`（无节点）移入 → 净 7→**6**。
- 18 段逐个判定：有节点 12 段 ={vad,asr,nlu,llm,tts(`PipelineFlow.kt:113` 核心 panel),command,wakeup,soul,memory,audio,echomuse,mcp}；**无节点进 global 恰 6 段 ={overview,presets,aiui,server,tuya,ota}**。
- 旧端权威文档逐字印证 `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md:49`（**此文件属 shared 侧，不在工程师并行修改的 `client/admin-web/` 三文件范围**）：「**全局配置**（id `settings`，[GlobalConfigLayout]）只保留能力总览、预设、AIUI 托管、服务、Tuya 接入、OTA；不重复挂载有流程归属的功能。」；`:71` 表格：「没有流程节点的部署级参数：能力总览、预设、AIUI 全链路、服务、Tuya 接入、OTA；与语音流程共用一份表单、一次 `POST /api/config`」。

## 3. 6 个全局段逐字盘点（段 label 来自 ConfigSections.kt，卡片 title 来自各 Card）
**① overview「能力总览」**(`ConfigSections.kt:29`，分组「总览`)
- 卡片 title `ConfigOverviewCard.kt:48`「能力总览」；两张只读子卡 `CapabilityOverviewCard`+`EngineHealthCard`(:39-41)，**0 可编辑字段**(:21-37)。
- 逐字：`:51`「无法读取能力清单」/`:52`「加载中…」/`:55`「刷新」/`:57`「刷新中…」/`:62`「（确认服务端地址可达后点「刷新」）」/`:66`「服务端未返回任何能力（注册表为空？请查看服务端日志）。」/`:68`「必需能力（缺失即启动失败）」/`:69`「可选能力（失败只降级）」。数据源 `GET /api/plugins` + `GET /api/health/engines`。

**② presets「预设」**(`ConfigSections.kt:30`，分组「总览」)
- 卡片 title `ConfigPresetCard.kt:57`「配置预设」；说明逐字(:58-60)：「把当前全局配置快照为预设，随时一键应用（回滚）。预设仅作用于全局配置；保存的是服务端盘上的生效值（不是表单草稿）。」
- 逐字：`:63`「预设名」/`:66`「如 night（字母数字与 - _ .）」/`:70`「保存中…」与「保存当前配置为预设」/`:85`「还没有预设。填好名称点「保存当前配置为预设」即可创建。」/`:107`「应用」/`:114`「删除」。
- 设备模式禁用形态(:47-51)：title「预设」+「预设仅作用于全局配置；当前正在编辑设备的个性化配置，请先点顶部「返回全局」再管理预设。」
- 语义(:35-41)：只快照盘上生效值≠表单草稿；应用成功后服务端覆盖全局配置 → 表单必须重载。

**③ aiui「AIUI 全链路模式」**(`ConfigSections.kt:41`，分组「服务与接入」)
- 卡片 title `ConfigInfrastructureCards.kt:93`「AIUI · 全链路模式」；开关 `:99`「启用 AIUI 全链路」。
- 互斥告警常驻(:108)逐字：「本地 ASR / LLM / TTS 全部闲置（云端闭环计费）；「人格」「记忆」等本地上下文能力不生效。」；未启用(:217)：「未启用：设备按本地级联流水线工作（SenseVoice ASR → 本地 LLM → TTS）。」
- 字段(启用后)：`应用 AppID`(:110)、`API Key`(:118)、`API Secret`(:127)、`场景`(:136)、`设备 SN 前缀`(:147 advanced)、`发音人`(:156)、`语速`(:166)、`音量`(:176)、`音调`(:186)、`人设提示词`(:196)、`上行分帧间隔`(:206 advanced)；两密钥带 `secretHint(...Configured)` presence(:124/:133)。整段回传、启用/切换**只能重启生效**(:89)。

**④ server「服务」**(`ConfigSections.kt:43`，分组「服务与接入」)
- 卡片 title `ConfigInfrastructureCards.kt:30`「Server · 服务」；`:32` 配置来源提示、`:34` `sectionNote("server")`。
- 字段：`监听端口`(:38「8000」)、`接入令牌`(:49)、`工作线程数`(:59 advanced)、`管理页目录`(:68 advanced)。
- 鉴权相关逐字(:55)：「设备与测试台连接时校验的令牌（hello 帧携带）。留空 = 不校验，仅建议在内网使用。」；`[server]` **只能重启生效**(:19/:26)。

**⑤ tuya「Tuya 接入」**(`ConfigSections.kt:47`，分组「服务与接入」) —— **C6 凭证不渲染的权威出处**
- 卡片 title `ConfigTuyaCard.kt:24`「Tuya · 设备接入」；类注释(:15-17)逐字：「只放两个部署级开关（启用 / 同步周期）；**云凭证不在这里**——唯一录入路径是 `POST /api/tuya/credentials`（设备管理中心的齿轮弹层），配置表单刻意不回传凭证键，两套 UI 不互相覆盖」。
- 字段仅 2 个：`启用涂鸦设备`(:30 默认 true :19)、`自动同步间隔（秒）`(:37 默认 86400 :43)。
- 逐字：`:33-34`「启用后开放涂鸦云目录同步与 /api/tuya/* 管理面；关闭即停用全部同步与管理面（保存即生效，无需重启）。局域网控制不依赖云凭证。」；`:46`「云凭证（区域 / access_id / access_secret）在「设备管理」页右上角齿轮里录入，这里不重复编辑。」；`:48`「未启用：/api/tuya/* 全部 404，设备管理页的涂鸦分区只显示未启用提示。」；`:54`「管理设备」(跳设备中心)。
- AGENTS.md:123 逐字再确认：「凭证（region/access_id/access_secret）只在齿轮弹层录入（`POST /api/tuya/credentials` 唯一路径，secret 只写不读）；`[tuya]` 开关与同步周期在「全局配置 → Tuya 接入」…配置表单刻意不回传凭证键，两套 UI 不互相覆盖）。该段全部热生效（保存即生效）。」
- 形态：2 控件 + 1 跳转按钮，**0 凭证字段**（刻意设计，admin-web 迁移必须保持）。

**⑥ ota「固件与 OTA」**(`ConfigSections.kt:49`，分组「服务与接入」) —— **0 配置字段动作面板**
- 卡片 title `ConfigOtaEntryCard.kt:39`「固件与 OTA」；两条 HintLine 逐字：`:41-43`「固件包托管走独立接口（`GET /api/ota/firmware`），不随 `POST /api/config` 回传：保存配置不会动固件，上传/删除固件也不会动配置。」；`:45-47`「上传与删除就在本页下方（唯一一套链路）：设备 OTA 时按「功能 + 型号」匹配，发错型号的包会直接变砖，所以删除必须二次确认。」
- 真正实现 `OtaFirmwareCard`(:51，来自 `OtaCard.kt`)；`OtaCard.kt:124` title「OTA 固件托管」、空态「尚未托管任何固件：上传后，设备下次 OTA 检查（开机 / 重连）即可拿到升级。」
- 形态：**0 个 `/api/config` 字段**，纯固件维护面(列表/上传/删除+二次确认)，走 `/api/ota/*`，与配置保存完全解耦(:21-24)。

## 4. State 与请求纪律（逐字文案，`ConfigFormActions.kt` / `ConfigPage.kt`）
- 加载 `GET /api/config`：`:57`「已加载配置」；`:60`/`:78`「加载失败: $errorMsg」。
- 配置来源 `GET /api/config/meta`(:16-26)：`:24`「配置来源：内置默认（未指定配置文件，保存不会持久化）」/`:25`「配置来源：$path ⚠️ 不在挂载卷上，重建容器会丢失！$hint」/`:26`「配置来源：$path（持久化 ✓）」。
- 保存 `POST /api/config`：三态 `ConfigPage.kt:249-251`；toast 必须接口返回后触发(:254-256)。
- hot 三档 `ConfigFormActions.kt:210-224` 逐字：`:212`「已保存并立即生效」(live)/`:213`「已保存，新会话生效」(next_session)/`:214`「已保存，下次启动生效」(restart_only)；`:219`「本次改动需：新会话生效」/`:220`「本次改动需：重启 server 生效」；兜底 `:224`「保存成功（新会话生效）」。注释:210「客户端只做翻译，避免"说立即生效、其实要重启"这类误导」。
- 409 `:238`「配置已被其他窗口修改，请重新加载后再保存」；通用失败 `:242`「保存失败: ${…}」。
- 预设 `:324`「请先填写预设名（字母数字与 - _ .）」/`:334`「已把当前配置保存为预设「$name」」/`:338`「保存预设失败: …」/`:351`「已应用预设「$name」（新会话生效）」/`:355`「应用预设失败: …」/`:368`「已删除预设「$name」」/`:371`「删除预设失败: …」。

## 5. 服务端权威：Settings **无独立端点**（明确结论）
`/api/config` 系列是**无条件基础路由**，非 feature 门控，与前端两个视图无关；所有「设置」都经它读写。
- 路由 `gateway/ws/mod.rs:158-189` 基础路由段（永远挂载）：`/api/config`、`/api/config/schema`、`/api/config/meta`、`/api/config/presets`(list/save/apply/delete + `POST /delete` 别名)。
- 鉴权：`/api/config` 系列 handler **无显式 `auth_ok`**、api router 无全局 auth layer；全 app 唯一中间件 `main.rs:175-178` `from_fn_with_state(engines, dev::guard)`。口径 = **局域网默认可管理**(`[server].expected_token` 空=不校验，印证 `ConfigInfrastructureCards.kt:55`)。唯一显式鉴权是 `db_query/db_backup` 的 `db_auth_ok`(`meta.rs:48-51`)。`dev.rs:97-143` blocked 路径(`/api/ws`、`/api/test*`、`/api/nlu/model/build*`、`/control`、`/data`、`/em/control`、`/em/data`、`*/inject`)——**`/api/config` 不在 blocked**，guard 仅注入 `x-xiaozhi-mode: dev`。
- `GET /api/config`(`config.rs:367-398`)：整份 Config + `revision`(FNV-1a 内容指纹 :350-364，基于打码后表示)；密钥打码 `SECRET_PATHS`(:74-86) 摘明文只留 `has_<leaf>` presence；`SECRET_ARRAY_PATHS`(:91) `llm.fallback.providers`；读库失败回退内存配置 **200**(:42-55，避免死局)。
- `PUT`/`POST /api/config`(`config.rs:437-534`)：**部分更新**(出现即覆盖/未出现保持/显式 null 清除，`merge_patch` :414-430)；`sanitize_patch`(旧路径重写+空串密钥删除+presence/revision 剔除)；`expected_revision`(或 `revision`)乐观并发，不符 → **409**(`:465` 逐字「配置已被其他窗口/端修改，本次保存已取消（避免覆盖别人的改动）。请重新加载配置后再改。」)；非对象 400 纯文本；合并失败 400 纯文本；workflow 预编译失败 400 JSON；无持久化落点 400；成功 200 `json_msg_full(ok,message,config_path,revision,impact)` 带 `hot`/`hot_hint`/`changed_sections`(:551-570)；`:508` 默认 message「配置已保存到 {}。TTS/LLM 引擎参数对**新会话**生效，ASR 等需重启 server。」。`SaveError` 6 变体 `config/save.rs:1-130`(RevisionConflict/Invalid/WorkflowInvalid/NoPersistence/Io/Persist)。
- `GET /api/config/meta`(`meta.rs:33-47`)：`{config_path,backend,loaded_from,persistent,hint}`；非挂载卷 hint 逐字(`meta.rs:41`)「⚠️ 配置库不在挂载卷上：重建容器会丢失（请挂载 ./server-data:/data）」。
- presets 4 端点(`config/presets.rs`)：GET list `{presets:[names]}`；POST save `{name}` 空名 400「预设名不能为空。」(:54) → 200「已把当前配置保存为预设「{name}」。」(:64)；POST apply 复用 merge_client_patch → 200「已应用预设「{name}」到全局配置（新会话生效）。」(:122) 带 impact/revision、404「预设「{name}」不存在。」(:95)；DELETE + `POST /delete` 别名 →「已删除预设「{name}」。」(:146)。失败：`:71`「保存预设失败: …」、`:112`「应用预设失败: …」(400 纯文本)、`:151`「删除预设失败: …」。
- `GET /api/config/schema`：`app_sections[].note` 是段级说明的**服务端唯一权威**（旧端 `ConfigInfrastructureCards.kt:34/:95`、`ConfigTuyaCard.kt:26` 均 `form.schema.sectionNote(id)` 下发，前端不硬编码）。

**1/2 结论**：旧端 Settings =「复用 Config 6 全局段的第二视图」，服务端 =「`/api/config` 系列无条件基础路由」，两者之间**没有任何 Settings 专属端点**。任何「为 Settings 新建端点/api」的规格都是虚扩。2/2 续报 admin-web 现状、重叠排查、实施清单与诚实化文案锚点。

---

---

> **以下为「报告 2/2」原文，全量保留，未删减。**

---

W13 / Settings 页前置研究 **报告 2/2（admin-web 现状 + 重叠排查 + 实施清单 + 诚实化文案锚点）**

路径基准同 1/2：admin-web = `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/client/admin-web/src/`；旧端 Kotlin = `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/client/shared/src/commonMain/kotlin/com/xiaozhi/admin/`。
约束遵守声明：除占位页内容外，本节**未把**工程师并行修改的 `client/admin-web/package.json` / `.prettierrc` / `AGENTS.md` 的状态变化当作事实；引用的 `nav.ts`、`router.tsx`、`SettingsPage.tsx`、`configSections.ts`、`configApi.ts`、`ConfigDraftContext.tsx` 均非被改三文件。1/2 引用的 AGENTS.md 是 `client/shared/.../admin/AGENTS.md`（旧端侧），亦不在被改范围。

## 6. admin-web 现状：settings 已是 nav 第 5 项占位，**不是第 11 项**
- `lib/nav.ts:14-75` `NAV_DESCRIPTORS` 共 **10 项**（W12 已落地 devicehub 第 3 项）；`settings` 是**第 5 项**(:39-44)：
  `{ key: "settings", label: "系统设置", description: "管理员账号、通知与服务运行参数。", tauriOnly: false }`
- `description`「管理员账号、通知与服务运行参数」与旧端「全局配置」**完全不符，是臆想文案**（旧端无管理员账号/通知/主题切换概念）；`label`「系统设置」也与旧端 label「全局配置」(`contracts/AdminNav.kt:58`) 不一致。
- 占位页 `pages/SettingsPage.tsx:4-14`（全文 15 行）逐字：
  ```
  <PlaceholderPage title="系统设置"
    description="管理员账号、通知与服务运行参数。"
    route="/settings"
    origin="Kuikly adminCore · 设置模块"
    notes={["本阶段不引入暗色主题切换，保持 shadcn 默认浅色与 GearUI 口径一致。"]} />
  ```
  notes 的「暗色主题切换」进一步坐实占位页把 settings 想象成外观/账号设置，与旧端语义无关；`origin`「Kuikly adminCore · 设置模块」也是占位措辞。
- 路由已通：`router.tsx:23` `settings: SettingsPage` 映射、`:37-45` 由 `NAV_DESCRIPTORS` 生成路由。
- `tauriOnly` 过滤：`nav.ts:77-80` `visibleNavDescriptors(tauriEnv)`；10 项中仅 `testbench` 为 true(:73)，settings=false → 浏览器/Tauri 均可见，**无过滤问题**。

## 7. 与已迁移模块逐项重叠排查：6 段 **100% 已实现**，需新增 0
`components/config/configSections.ts:20-39` `CONFIG_SECTIONS` 18 段**全部 `migrated: true`**；`:16` 注释逐波次逐字：「C0 总览与预设、W4/C1 server / aiui / audio（playback 挂 audio 页尾）、W5/C2 vad / asr / command / wakeup、W6/C3 llm / tts、W7/C4 nlu、W8/C5 soul / memory、W9/C6 echomuse / tuya / mcp（只读）、W10/C7 ota（动作面板，0 配置字段）」。`ConfigPage.tsx` 已渲染全部 18 段组件。

| # | 全局段 id | 旧端逐字 label | admin-web 已实现于 | 波次 | 判定 |
|---|---|---|---|---|---|
| 1 | overview | 能力总览 (`ConfigSections.kt:29`) | `components/config/ConfigOverviewSection.tsx` | W3/C0 | 已实现 |
| 2 | presets | 预设 (:30) | `ConfigPresetsSection.tsx` | W3/C0 | 已实现 |
| 3 | aiui | AIUI 全链路模式 (:41) | `AIUISection.tsx` | W4/C1 | 已实现 |
| 4 | server | 服务 (:43) | `ServerSection.tsx` | W4/C1 | 已实现 |
| 5 | tuya | Tuya 接入 (:47) | `TuyaSection.tsx` | W9/C6（只读） | 已实现（**凭证 3 项刻意不渲染，须保持**） |
| 6 | ota | 固件与 OTA (:49) | `OtaSection.tsx` | W10/C7（动作面板，0 配置字段） | 已实现 |

- **结论：Settings 需新增 0 配置字段 / 0 端点 / 0 api 封装 / 0 表单原语。** 唯一真实工作是「入口与编排」。
- **双 UI 覆盖风险（C6 教训）**：`TuyaSection.tsx`（W9/C6 标注「只读」）须与旧端 `ConfigTuyaCard.kt:46`、AGENTS.md:123 一致 —— 凭证 region/access_id/access_secret **只能在设备中心齿轮弹层录入**(`POST /api/tuya/credentials`)，且 secret 只写不读、无值可显。若 Settings 复用 TuyaSection，**绝不能为「看起来完整」补渲染凭证**，否则两套 UI 覆盖。同理 `OtaSection.tsx` 是动作面板、0 配置字段，不能塞回 config draft。
- **边界提醒**：admin-web `CONFIG_SECTIONS` 是 18 段平铺（含 12 个有流程节点的段），旧端 settings 只暴露 6 段。若 Settings 做成「再列一遍 6 段卡片」，会与 `ConfigPage` 的 6 段形成**同一组件的第二份挂载** —— 需明确二者是「同一 `ConfigDraftProvider` 下的两个路由视图」而非两份独立状态（见 §9 拍板项）。

## 8. api 封装检查单落地预判：全部已存在，无需新增
`lib/configApi.ts`（280 行）逐项（行号来自本轮 grep）：
- `getConfig`(:202)、`putConfig`(:210)、`ConfigConflictError`(:142，带 `currentRevision`；:224 把 409 转成它「**不吞掉**」见 :208 注释)、`configErrorMessage`(:175)、`saveImpactOf`(:186)、`getConfigSchema`(:247)、`getConfigMeta`(:250)、`listPresets`(:253)、`savePreset`(:259)、`applyPreset`(:268)、`deletePreset`(:275)。
- `context/ConfigDraftContext.tsx:1-14` 设计要点：baseline/draft/revision 分层；`diffPatch` 只提交变化叶子；密钥明文只存 `secretValues` 绝不进 draft；**保存成功后必须回读 `getConfig()`**（否则「清空密钥」假成功）；409 保留 draft 置 conflict、不自动重载。
- **检查单结论**：Settings 落地**不产生任何新 api 函数**。需复用的既有能力：`getConfig` / `getConfigMeta` / `getConfigSchema`(段级 note) / `putConfig`(含 409) / presets 五个 / `ConfigDraftProvider` + `SaveBar` / `configErrorMessage` / `saveImpactOf`。若 Settings 另起一套 draft，会与 `ConfigDraftProvider` 冲突 —— 应共用。

## 9. 实施清单 + 需 team-lead 拍板的形态项
**必须先拍板（否则工程师无法开工）**：既然 6 段已 100% 实现于 ConfigPage，Settings 形态只可能二选一：
- **方案 A（推荐，最贴合旧端）**：Settings = **ConfigPage 的「全局段过滤视图」** —— 同一 `ConfigDraftProvider`、同一 `SaveBar`、同一批 6 个 Section 组件，仅左菜单只列 6 段（按旧端 `ConfigSections.globalGroups` 分组：总览 / 服务与接入）、顶部共用「保存配置」。相当于把旧端 `GlobalConfigLayout`(`ConfigWorkspaceViews.kt:232-254`) 的编排搬过来。工作量：新增 1 个视图壳 + 路由/菜单编排，**0 新表单、0 新端点**。
- **方案 B（更省，但需诚实标注）**：Settings = **指向 ConfigPage 的纯跳转/锚点面板**（「0 配置字段纯跳转面板」，明确告知用户「全局配置在配置管理页」），只修占位页文案。好处是绝无双 UI 风险；代价是与旧端「侧栏独立菜单直接编辑」体验不完全等价。
- 无论 A/B，**不要新建第 3 份表单 / 第 3 套 draft**。

**确定要改（与形态无关）**：
1. 修 `lib/nav.ts:40-44` settings 描述符：`label`「系统设置」→ 建议对齐旧端「全局配置」（或保留但 description 必改）；`description`「管理员账号、通知与服务运行参数。」→ 改为旧端真实语义，建议逐字取 AGENTS.md:71「没有流程节点的部署级参数：能力总览、预设、AIUI 全链路、服务、Tuya 接入、OTA；与语音流程共用一份表单、一次 `POST /api/config`」（可精简）。
2. 修 `pages/SettingsPage.tsx:7-11` 占位文案：删掉「管理员账号、通知」「暗色主题切换」「Kuikly adminCore · 设置模块」等与旧端无关的臆想内容；`origin` 改真实来源（如「Kuikly adminCore · ConfigWorkspaceViews.GlobalConfigLayout」）。
3. 保持 `tauriOnly: false`（现状正确，无需改）。

**必须保持的边界（回归检查点）**：
- Tuya 凭证 3 项不渲染（C6）；`OtaSection` 0 配置字段、不进 draft；保存仍是一次 `POST /api/config`（部分更新 + `expected_revision`），不是逐段即时保存。
- `router.tsx:23` 映射已有，无需改；`NAV_DESCRIPTORS` 保持 10 项，settings **不新增为第 11 项**。

## 10. 诚实化文案锚点（逐字中文原文，供实施/QA 对照）
1. **hot 三档（禁止一律说「已生效」）**：`ConfigFormActions.kt:212-214`「已保存并立即生效」/「已保存，新会话生效」/「已保存，下次启动生效」；服务端 `config.rs:551-554` 同源，`hot` 取注册表最严格档；兜底 `:224`「保存成功（新会话生效）」。
2. **409 冲突（禁止自动覆盖/静默重载）**：服务端 `config.rs:465`「配置已被其他窗口/端修改，本次保存已取消（避免覆盖别人的改动）。请重新加载配置后再改。」；旧端 `ConfigFormActions.kt:238`「配置已被其他窗口修改，请重新加载后再保存」；admin-web `configApi.ts:142/:224` `ConfigConflictError` + `ConfigDraftContext` 保留 draft 置 conflict。
3. **密钥 presence（禁止把回读 `has_*` 当明文 / 禁止「清空密钥」假成功）**：`config.rs:74-91` `SECRET_PATHS`/`SECRET_ARRAY_PATHS`；`ConfigDraftContext.tsx`「保存成功后必须回读 `getConfig()`」；旧端 `secretHint(...Configured)`(`ConfigInfrastructureCards.kt:124/:133`)。
4. **配置来源 / 未持久化警告**：`meta.rs:41`「⚠️ 配置库不在挂载卷上：重建容器会丢失（请挂载 ./server-data:/data）」；`ConfigFormActions.kt:24-26`「配置来源：内置默认（未指定配置文件，保存不会持久化）」/「…⚠️ 不在挂载卷上，重建容器会丢失！…」/「…（持久化 ✓）」。
5. **预设不等于表单草稿**：`ConfigPresetCard.kt:58-60`「保存的是服务端盘上的生效值（不是表单草稿）。」；设备模式禁用 `:48-50`「请先点顶部「返回全局」再管理预设。」
6. **OTA 与配置解耦**：`ConfigOtaEntryCard.kt:41-43`「保存配置不会动固件，上传/删除固件也不会动配置。」；`:45-47`「发错型号的包会直接变砖，所以删除必须二次确认。」
7. **AIUI 互斥（禁止只写进 help）**：`ConfigInfrastructureCards.kt:108`「本地 ASR / LLM / TTS 全部闲置（云端闭环计费）；「人格」「记忆」等本地上下文能力不生效。」
8. **Tuya 凭证唯一入口（C6）**：`ConfigTuyaCard.kt:46`「云凭证（区域 / access_id / access_secret）在「设备管理」页右上角齿轮里录入，这里不重复编辑。」；AGENTS.md:123「secret 只写不读」「两套 UI 不互相覆盖」。
9. **能力总览空态不是「全绿」**：`ConfigOverviewCard.kt:66`「服务端未返回任何能力（注册表为空？请查看服务端日志）。」
10. **段级说明以服务端为准**：`ConfigInfrastructureCards.kt:34/:95`、`ConfigTuyaCard.kt:26` 均 `form.schema.sectionNote(id)`；不要在 admin-web 硬编码段说明。
11. **toast 时机**：`ConfigPage.kt:254-256`「toast 在**接口返回后**由 save 回调触发」—— 保存结果文案不能前端抢答。

## 11. 给 team-lead 的三句话总结
1. **口径纠正**：不是「第 11 项」也不是「8 全局段」—— settings 已是 nav 第 5 项占位，旧端真实全局段是 **6 段**（8 任务口径 → 7 `ConfigSections.kt:60-61` 旧注释 → 6 代码；AGENTS.md:49/:71 与 `PipelineFlow.primaryForSection` 双向印证；echomuse/mcp 因加入流程节点移出、tuya 新增移入）。
2. **形态**：旧端 settings = 复用 Config 6 段的第二视图（共用 `ConfigFormState`/draft/`SaveBar`/一次 `POST /api/config`），服务端 **0 独立端点**；admin-web 6 段 **100% 已实现**于 ConfigPage，需新增 **0 字段 / 0 端点 / 0 api**。
3. **唯一决策**：拍板 Settings 形态（A 过滤视图 / B 纯跳转），并修占位页与 nav 的臆想文案；无论哪种都保持 Tuya 凭证不渲染、`OtaSection` 0 字段、共用 draft，避免 C6 双 UI 覆盖。

（报告 1/2 + 2/2 完毕；全程只读，未改任何文件、未构建、未 git 操作。）
