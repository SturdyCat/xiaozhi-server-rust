# 语音主流程重组、节点测试与公共 UI

> 当前实施记录。信息架构和接口已接线；最终验证状态见末节，不将编译、合成接口响应或测试夹具等同于模型/真机验收。旧 UI 设计记录保留在[历史文档](<admin-ui-design.md>)。

## 1. 主流程是入口，不是插件平铺

公共管理台默认进入「语音流程与配置」，核心节点固定为 **VAD → ASR → NLU → LLM → TTS**。定义集中在[核心面板描述符](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/PipelinePanels.kt>)与[流程描述符](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/PipelineFlow.kt>)，渲染分支显式编译，不下载执行 UI 插件。

扩展位置支持开始、结束、核心节点之前与之后；排序稳定且重复 ID 立即失败。目前展示输入、唤醒、会话指令、人格、记忆召回、输出与记忆写回。记忆召回/写回的节点身份不同，但复用一个配置源；同理输入/输出复用音频配置，而非创建不同步的副本。

提供方实现、上下文增强、AIUI 托管全链路、终端适配和基础设施仍可分别配置，不把它们误称为第六个核心阶段。AIUI 的外部闭环不能被本地级联预览冒充。

[路由草稿所有者](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigFormState.kt>)只维护一个 `ConfigSelection(sectionId,nodeId?,workflowPositionId?)`。「语音流程与配置」内容区顶部及独立配置页共用[横向工作流](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/PipelineWorkflowView.kt>)（不跨运维页面），五个模块间及头尾共六个可点击圆点；窄屏横向滚动，不变成纵向步骤或下拉选择。位置由[纯模型](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/PipelineWorkflow.kt>)从现有 anchors 投影，12 个真实节点各归属一次：

| 圆点 | 下方扩展菜单 |
|---|---|
| 开始 | 终端与音频入口、唤醒与问候 |
| VAD → ASR | 暂无扩展 |
| ASR → NLU | 会话指令 |
| NLU → LLM | 人格与提示词、记忆召回 |
| LLM → TTS | 暂无扩展 |
| 结束 | 终端输出适配、轮次结束与记忆写回 |

模块/圆点点击后进入[节点管理工作区](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigTabs.kt>)：只有「配置 / 测试」两个官方 Tabs（文字下官方滑块指示条），每个 tab 均为左菜单、右内容。核心配置菜单为模型与参数、运行状态；圆点配置菜单选择所属功能，测试菜单选择单个共享动作。空圆点没有伪配置、能力或 API 动作。全局设置与运行观测仍可达；无配置搜索框。不恢复第二份 active 状态。切 tab、状态页或测试动作只改变局部展示；右内容重建键保留 section/revision/node/position/panel，草稿和请求所有权不复制。

动作行统一右对齐、允许窄屏换行；小于 700 宽的工作区左菜单保留 92 宽、卡片内边距减为 12。保存仍在右上方；目录加载、凭据校验和当前草稿预览继续保留其原有作用域，不伪装成保存态节点测试。

## 2. Web 与 macOS 的测试边界

[公共测试面板](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/NodeTestPanel.kt>)使用同一[控制器](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/NodeTestsState.kt>)和[纯状态模型](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/NodeTestModels.kt>)：相同输入验证、快照、异步状态、错误、计时口径与陈旧回包处理。

测试是否可执行取服务端能力、客户端适配器及权限/设备就绪的交集。平台名称不是可用性证据。公共面板尚未声明本地录音/播放适配器，故不会在 Web 露出硬件动作，也不会在 macOS 公共面板假装已接入；已有原生测试台保留真实麦克风/Opus/播放桥。

入口节点提供文本与音频串联预览；核心节点与各扩展拥有自己的测试动作。服务端没有安全实现的功能显示明确不可用原因，而不是成功占位。当前记忆读写测试因存储工厂会创建/迁移或后台建索引而失败关闭，不借用生产存储伪装只读隔离。

### 保存态快照与 API

[测试服务](<../server/src/app/bench_api.rs>)从 SQLite `ConfigStore` 取一个保存态快照及字符串 revision；不从启动配置或磁盘 TOML 回退，不保存、发布生产引擎、写入历史、写记忆或发送设备动作。

- `GET /api/test/capabilities`：`source`、`revision`、动作可用性/原因/就绪口径/输入类型、样本目录和资源边界。`configured` 仅表示允许调用，不等于已推理成功。
- `POST /api/test/run`：稳定 `action_id`、可选 `text`/`sample_id`/受限 PCM16、`expected_revision`。不接收任意路径/URL，不接受草稿或设备 overlay 冒充已保存快照。
- 结果保留阶段原有字段，并附 `request_id`、`action_id`、`source`、`revision`、`context_mode`、阶段数组和明确错误码。未知状态不能升级成成功。
- 文本串联显式跳过 VAD/ASR；本地级联预览标为 `pipeline_preview`，不是生产托管全链路的复刻。
- 旧 `/api/test/{llm,tts,vad}` 与模型目录保留；请求级兼容覆盖只影响本次调用，并用不同 `source` 说明，公共新面板不发送这些覆盖。
- 旧、新测试 HTTP 端点均经过同一管理鉴权，不因「只是测试」开放模型资源。精确上限、请求形状与样本来源见[样本和测试契约](<node-test-samples.md>)及[契约实现](<../server/src/app/bench_api/contracts.rs>)。

提供方首 token/首 PCM、socket 完成写入和设备真正播放是三个不同观测点。公共面板标明实际测量范围，不把合成完成说成播放成功。

### 鉴权和代际

[管理访问助手](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AdminAccess.kt>)只读取实际 Pager 请求凭据，构建独立 Bearer 头；绝不拿配置草稿 `server.expected_token` 当当前管理身份。Web 沿用启动参数，macOS 服务器设置提供密码输入，仅保留在本次 Pager 会话内，不写 SP、配置正文或日志。

切服务器或显式修改凭据会递增非秘密 access epoch；配置重载、revision、设备目标和节点切换也使旧请求失效。A→B→A 不是相等即有效。忙碌动作拒绝重复触发；过期回包只释放/更新自己的代际，不清掉更新请求。客户端丢弃回包不宣称远端推理已经物理取消。

409 保留配置草稿与测试输入，需显式刷新/重载；测试刷新本身不提交配置。保存态测试区域遇到未保存草稿会显示提示。

## 3. 公共控件与 SVG 图标

[纯 UI 契约](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/UiContracts.kt>)区分可用性、动作阶段/结果、选择、混合勾选及业务继承三态，不能用一个 loading/checked 字段混装。

[按钮封装](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AdminButtons.kt>)保留官方 Button 的交互、标题与无障碍语义；SVG 是不可交互的兄弟视图，而不是图片属性或 Button 内任意子节点。加载指示器同样是兄弟 Canvas，使用声明式旋转动画并随生命周期移除，不用定时器旋转字符。静态图标和 spinner 共用一个位置，选择态与动作成功态独立。

19 个 `IconToken` 的[原创 SVG 源文件](<../client/shared/src/commonMain/assets/common/icons/source/>)**仅在 common 维护一份**，只用 24×24 viewBox 描述逻辑坐标，不设置根 `width`/`height`；显示尺寸完全由容器控制，内部矢量等比居中撑满。资源使用 `currentColor`，通过[图标入口](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AdminIcons.kt>)调用独立的[公共 SvgView](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/SvgView.kt>)。Web 构建从同一 common 资源生成内联资源表并编入壳 bundle，不再复制第二套 `.svg` 到 Web 目录。普通位图继续使用官方 Image；SVG 不进入图片加载器，不派生 PNG，也不栅格化为 UIImage。来源、CC0 许可及 SVG 内容哈希在[清单](<../client/shared/src/commonMain/assets/common/icons/manifest.json>)。

Apple 端的 [XzSvgView](<../client/apps/macosApp/XzSvgView.m>)实现固定版本渲染器的原生 View 协议：可信 Bundle SVG → [有界文档模型](<../client/apps/macosApp/XzSvgDocument.m>) → [CGPath](<../client/apps/macosApp/XzSvgPath.m>) → 每视图独占的 CAShapeLayer 树。只缓存不可变路径/样式文档，不缓存位图或跨视图共享可变 layer。viewBox 按 bounds 等比居中，缩放不重建位图；tint 只映射 `currentColor`，保留显式颜色。UIKit 实现可由 iOS 与 Mac Catalyst 宿主复用，不增加第三方 SVG 引擎。

Web 的[同名扩展视图](<../client/apps/h5App/src/jsMain/kotlin/com/xiaozhi/admin/WebSvgView.kt>)使用受限 SVG DOM 与相同公共属性；[资源入口](<../client/apps/h5App/src/jsMain/kotlin/com/xiaozhi/admin/TrustedSvgDocuments.kt>)只读取构建时从 common 自动生成的内联资源表，不额外 fetch SVG，也不使用 img、canvas 或图像滤镜模拟图标。源限于 `assets://common/` 下安全路径的内置 SVG；不接受远程 URL、任意文件、路径穿越或可执行内容。首版覆盖 svg/g/path/rect/circle/ellipse/line/polyline/polygon、viewBox、基础填充/描边、透明度与变换；路径支持相对/绝对命令及弧转贝塞尔。渐变、clip/mask、filter、foreignObject、脚本、动画及复杂 CSS/文字布局未实现时明确拒绝，**不回退为位图**。完整 App 资源装包与页面显示仍需独立验收。

## 4. 配置与音频不变量

SQLite 与注册表仍是配置和热生效元数据的唯一权威。草稿只维护一次，密钥 presence/空白不发送语义不变；overlay 仍提交白名单差量，保存不会覆盖基线。

[TTS 引擎租约](<../server/src/engine/tts.rs>)将实际实例、构造配置、指纹和 generation 一起发布；失败或过期构建不能发布未经构建的身份。会话和缓存取实际租约，而非最新请求配置。语言模型按 generation/语言单飞构建，不在全局锁内阻塞构造；失败可重试。

[语音段处理](<../server/src/app/session/utterance.rs>)区分 Speech 与 NoSpeech：前者 flush/reset 后识别，后者丢弃/reset，不把噪声或旧残留送 ASR。EchoMuse 显式结束后重备下一轮，重复结束幂等、旧 control/data 代际不能结束新轮次。ESP 协议边界保持独立。

播放场景配置是独立 `[playback]`，输出节点复用[公共播放配置卡](<../client/shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigPlaybackCard.kt>)。交互保持既有协商与起播策略，播报只加深有上限的服务端起播蓄水，不扩充固件队列。具体生效档位取服务端 schema，不在客户端另建热配置表。

音频重构必须保留串行有界 TTS 预取、阻塞推理闸门、仅完整成功缓存、整次回复绝对节拍、ESP 首帧保护与取消/epoch。编码、采样率、声道和帧时长属于终端媒体类型；不能把 Echo PCM 伪装成 Opus。socket 确认不等于设备播放确认。

## 5. 布局调整前的已完成基线

2026-10-21 早先版本的基线验证如下；其中旧 54 项浏览器报告对应旧布局，不作为本次横向工作流的验收：

| 范围 | 实际结果与边界 |
|---|---|
| Rust 全量默认 feature 回归 | **786 passed、0 failed、3 ignored**；另以 `DOCS_RS=1` 检查 `sherpa,nlu` 通过，无警告；不代表原生链接或模型推理 |
| 客户端纯模型 / BLE | **16 / 15 项全通过**；最终使用 `--rerun-tasks` 实际重跑，直接编译生产纯逻辑，不复制实现、不伪造 Kuikly 宿主 |
| 跨端构建 | 公共 macOS Kotlin 编译、Release Framework 链接与 Catalyst 平台补丁、H5 `publishWeb` 均通过；保留现有 expect/actual Beta 与 webpack 包体积提示 |
| SVG 唯一源及产物 | **19 个 common 源 SVG，共 6,073 字节**；根无固定宽高，生成 registry 的字符串逐字节匹配源文件，业务 ZIP 资源匹配且无旧 PNG；Web 目录 `.svg` 文件为 **0** |
| Apple 实际渲染器探针 | **1,174 项断言通过，19 个资源、28 个形状**；真实 Mac Catalyst UIKit + 固定 Kuikly archive，无 SDK shim。验证实际工厂/属性派发、24/48/128/500 缩放、CGPath 复用、独占 layer、动态 tint/复原、几何及安全限额；严格警告即错误编译/链接通过 |
| 真实 Chrome 155 | 最新双 bundle 的 **54/54 项通过**；详见[完整报告](<../client/build/ui-validation/final-common-only-svg/report.json>)，记录产物哈希及截图。只读合成夹具不执行模型、不持久化配置 |

浏览器覆盖 320/375/768/900/1440 宽度与宽窄切换、官方下拉选择关闭、12 个流程节点、Web 硬件入口隐藏、输入/异步结果、真实 spinner 旋转、防重复请求、陈旧回包隔离、测试与配置 409 保留输入。已修复并复验 resize 转发。另验证容器控制 SVG 实际尺寸、`currentColor` 跟随 tint、无图像/位图图标、无 Web SVG 副本与 SVG HTTP 请求；浏览器克隆几何缩放检查不冒充原生层结构证明。无未捕获异常或非预期资源失败；两个 HTTP409 是预定夹具响应。

**未证明**：真实模型推理、受保护真实服务联调、macOS 完整 App 打包/运行、原生 IME、真实录音/播放、Echo/ESP 真机与网络抖动听感。默认 feature 的音频桩与 `DOCS_RS=1` 类型检查均不是这些能力的证明。Web 与 Apple 共用受限功能契约，但不宣称极端弧/控制点的底层数值边界完全一致，也未把 Web 路径负例的代码审查当作运行测试。

验证命令沿用[服务端指引](<../server/AGENTS.md>)和[客户端指引](<../client/AGENTS.md>)；真实浏览器入口为[UI smoke harness](<../tools/admin_ui_browser_check.mjs>)，Apple 结构验证入口为[原生探针脚本](<../tools/check_svg_native.sh>)及[测试源](<../tools/svg_native_probe.m>)，资源/生成表/ZIP 的逐字节检查入口为[SVG 校验器](<../client/shared/src/commonMain/assets/common/icons/source/verify.py>)。这些检查不接触 DSH GUI。

## 6. 本次配置区工作流验收

2026-10-21 最终修订：工作流从全局顶部移入「语音流程与配置」内容区（运维页不再渲染），移除配置搜索（`ConfigSections.search`/keywords 一并删除），Tabs 改用官方 `indicatorInTabItem` 文字下滑块（44×3 圆角 accent pill，`indicatorAlignCenter`）。未改动 Rust 业务逻辑；最终证据为[第四轮 Chrome 报告](<../client/build/ui-validation/workflow-top-v4/report.json>)。

| 范围 | 本次实际结果 |
|---|---|
| 纯模型与协议 | 管理台 **24 项**（含 8 项工作流投影；搜索断言随功能删除）及 BLE **15 项**随最终构建实际执行并通过 |
| 最终跨端产物 | shared JS、macOS Kotlin 编译、Release Framework 链接及 Catalyst 补丁、H5 `publishWeb` 通过；仍有既有 expect/actual Beta、Gradle 与 webpack 包体积提示 |
| Chrome 155 | **242/242 通过**（含 8 处新增滑块几何断言）；0 未捕获异常、0 非预期资源失败，自管 Chrome 与 fixture server 均退出 |
| 布局与交互 | 工作流仅出现在配置内容区（概览页断言无工作流，经侧边栏「语音流程与配置」可达）；320/375/768/900/1440 的五模块、六圆点、双 Tabs（官方滑块）、左菜单/右内容、右对齐动作、空圆点无请求全部验证；Admin 800/900 扣 241 后 92 宽菜单、独立 900 为 192 宽菜单 |
| 补充回归 | scrollport 不裁切、隐藏轨道后原生横向滚轮生效、紧凑冲突标题不与按钮重叠、健康修复从 Testing 回 Configuration 参数、SVG 唯一源、异步防重/陈旧响应、409 草稿保留均通过 |
| 产物 | `index.html` afef51e2…、`nativevue2.js` 4a31475d…、`h5App.js` 3b873830…（sha256，报告内逐一核对） |

[宽屏独立配置页](<../client/build/ui-validation/workflow-top-v4/config-1440.png>)、[宽屏管理台](<../client/build/ui-validation/workflow-top-v4/admin-1440.png>)、[窄屏测试](<../client/build/ui-validation/workflow-top-v4/workspace-test-320.png>)、[紧凑冲突操作](<../client/build/ui-validation/workflow-top-v4/config-409-narrow-320.png>)与[健康修复](<../client/build/ui-validation/workflow-top-v4/admin-health-repair.png>)均为实际发布产物的浏览器截图。上一轮（全局工作流，v1–v3：182→237 项）的迭代与修复记录保留在对应报告目录。

**边界与已知观察**：浏览器仅使用只读合成 HTTP 夹具，不运行模型、不持久化配置；macOS 仅验证共享编译与框架，不代表完整 App 启动、原生录音/播放或真机验证。两条预期 HTTP409 后的 JSONEngine 空字符串解析 console 日志原样保留，不宣称 console 零错误。既有单行长 Toast 在 320 宽仍可能裁切文字，未包含在此次标题/交互控件修复中，亦不将回归通过等同所有像素完美。

本任务未调用 `git add` / `git commit`，未对既有暂存与任务无关变更进行重置或清理。
