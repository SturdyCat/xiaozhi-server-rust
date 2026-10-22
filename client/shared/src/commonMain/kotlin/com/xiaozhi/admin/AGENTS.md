# 公共管理台维护

这里是跨端管理页面、表单和网络状态；macOS 专用测试台不属于 commonMain。

**单一壳**：web 与 macOS 共用 `@Page("admin")`（[AdminPage](<AdminPage.kt>)），平台差异全部收口到 [AdminPlatform.kt](<AdminPlatform.kt>) 的 expect/actual（macosArm64Main 真实现，js/android/ios 空实现）。原 macOS 专属 `AdminShell` @Page("router") 双壳已废除。

## UI 与布局

- [AdminTheme](<AdminTheme.kt>)提供 token，现有[组件](<AdminComponents.kt>)、[表单控件](<AdminFormControls.kt>)、[widgets](<AdminWidgets.kt>)统一封装官方控件；不要绕过封装散写样式。
- 页面/配置分区由[AdminNav](<AdminNav.kt>)和[ConfigSections](<ConfigSections.kt>)常量表驱动，id 稳定、order 确定。新增分区同时补渲染分支。
- Tabs/PageList 显式提供 pageItemWidth/Height；滚动事件回传 Tabs.scrollParams，点击 tab 调 scrollToPageIndex。不自画指示器。配置/测试工作区无横向 PageList，以相同两态驱动官方 Tabs.scrollParams 与选中内容。
- 紧凑断点取 `adminContentWidth`：AdminPage 常驻侧栏时扣 240+1，其余页取整窗，不能直接拿整窗判工作区宽。H5 官方 Scroller 的 `showScrollerIndicator(false)` 对应 `isList list-no-scrollbar`；宿主 [index](<../../../../../../../apps/h5App/web/index.html>)须保留该 class 的 scoped CSS，不能用 overflow:hidden 或 DOM 轮询伪修滚动条裁切。
- 嵌套 DSL 先 `val ctx=this` 再显式访问 Pager；复用渲染为文件级 ViewContainer 扩展，ViewBuilder 显式传接收者。observable 在 attr/vif 闭包内读取才被跟踪。
- 可点击目标（侧边栏项/标签页/按钮/下拉与选项行/健康跳转行）一律 `attr { cursor(CursorType.POINTER) }`；core 的 cursor prop 仅 macOS 原生直读，web 端靠 h5App 的 `WebCursorPropHandler` 外部处理器落 `style.cursor`，别在业务里散写 CSS。
- 左右分栏（左菜单 │ 1px 纵向分隔线 │ 右面板）统一走 [AdminComponents](<AdminComponents.kt>) 的 `menuPaneRow`/`dividerV`（配置工作区 ConfigTabs 与 EchoMuse 设备卡卡内同一出处），不自画第三条竖线。web 端文本划选复制依赖 h5App 入口 `KuiklyProcessor.preventDefaultSelect = false`（渲染器默认在桌面浏览器把 Scroller 内 selectstart 全部拦截，整页只剩 Scroller 外的标题可选）；drag 拦截保持 true 防 HTML5 原生拖拽卡死 PageList，不要回退。
- 官方传统 DSL Button 保留标题、交互与无障碍语义；加载 Canvas 与图标 Canvas 都是不可交互的兄弟覆盖层，不塞任意子节点、不交给图片槽。图标 = [AdminIcons.kt](<AdminIcons.kt>) 内嵌 SVG pathData（viewBox 0 0 24 24）经 Kuikly Canvas 即时绘制（方法对齐 xiaoya-player 的 TvIcons，2026-10 从 assets 资产迁入代码：原三端宿主实现随 macApp 资源缺包全空白而废除）；新增/改图标直接写 pathData（[AdminIconVectors.kt](<AdminIconVectors.kt>) 解析器支持 M/L/H/V/C/S/Q/T/A/Z），IconToken↔几何一一对应由 `:shared:verifyAdminIcons` 护栏锁定，禁止回退资源文件/PNG/UIImage/位图。NMAllResponse.data 非空，不复制恒假的 data==null 判断。

## 表单与网络契约

- **异步回调线程契约**：Kuikly 的 Kotlin 侧只允许在 context 线程碰 observable/`setTimeout`。mac 端管理台实时通道 [AdminRealtime](<AdminRealtime.kt>) 的 `PlatformWsSocket` actual 经 XiaoZhiModule 原生桥（`XiaoZhiModule+AdminWs.m`）实现——模块回调 block 由框架包裹 `performOnContextQueue`，任意原生线程 invoke 都会落回 context 线程；**禁止**在平台 actual 里自建 NSURLSession/GCD 直调共享层（2026-10 断线重连 `assertContextQueue` 卡死事故根因）。服务器设置（`macServerSwitchEntry`）同理：发现事件走原生桥回调，不在 Kotlin 侧起线程。
- [ConfigFormState](<ConfigFormState.kt>)保存草稿，[fill](<ConfigFormFill.kt>)映射读取；密钥只读 presence、写入 putSecret，空草稿不发送。新增密钥与服务端 SECRET_PATHS 同批改。
- 整型字段用 labeledIntField、浮点字段用 labeledDecimalField（[AdminFormControls](<AdminFormControls.kt>)，输入侧实时剥离非法字符）：裸 labeledField 留给文本/密钥——整数字段混入脏输入会让 `toIntOrNull() ?: 默认值` 把错误静默存成默认值。
- BLE 的危险目标 handle 是严格校验例外：使用 `labeledField` 保留原始输入并由 [EchoMuseBleCodec](<EchoMuseBleCodec.kt>) 的纯函数拒绝非法十进制/范围；不得把 -1 或 1.2 清洗成别的真实外设 handle，也不允许解析服务树数字时截断小数。
- hot/字段提示由[FormSchemaState](<FormSchemaState.kt>)消费服务端 schema，不手写另一套档位。schema 不是自动布局，engine/preset 可选项仍需两端同步。
- 409 保留草稿，显式“重新加载”，不静默覆盖；紧凑标题栏冲突/设备操作另起右对齐行，不挤压标题。preset 应用复用服务端配置管线；overlay 编辑只提交设备增量，不能混成全局整份配置。健康修复/全局设置的明确配置跳转用 `openConfigSection` 回到 Configuration 参数页，普通顶部节点点击才保留 tab。
- NetworkModule 只支持 GET/POST，使用服务端 POST 别名；curl 的 PUT/DELETE 能用不等于页面可用。
- 人格预览 effective 全段 fill，客户端不复制预设合并。能力 enabled 与 phase 分开，异常/降级须显示原因与修复指引。
- 设备/会话/最近对话/延迟注明进程内口径，online 非心跳，不呈现为完整历史。

本级说明 · [客户端构建](<../../../../../../../AGENTS.md>) · [管理台界面设计与验收记录](<../../../../../../../../docs/designs/admin-ui-design.md>)


---

## 管理台页面与操作

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

### 5. 管理台（Web + macOS）

管理台是 **Kuikly 多端工程**（`client/`），逻辑在 `client/shared/commonMain` 跨端复用；web 构建产物由 server 在 `/` 静态托管。

默认落地页是 **语音流程与配置**（`@Page("admin")`）：该内容区顶部为[横向工作流](<PipelineWorkflowView.kt>)（仅配置区渲染，不跨运维页面），VAD → ASR → NLU → LLM → TTS 为模块，头尾及衔接共六个可点击圆点；独立配置页复用同一组件。轨道宽按 `adminContentWidth` 自适应（桌面 ≥1280 无横向滚动，不设最大上限），窄屏横向滚动，不能退回纵向节点轨道/下拉。点击模块/圆点进入「配置 / 测试」两个官方 Tabs（官方 `indicatorInTabItem` 文字下滑块样式），各自固定左菜单、右内容；测试左菜单选择单个动作。无配置搜索框，分区只能经工作流、全局设置或侧边栏到达。工作区小于 700 宽保留 92 宽左菜单，操作行靠右并可换行。布局见 [ConfigTabs](<ConfigTabs.kt>)及[工作区菜单](<ConfigWorkspaceViews.kt>)，位置投影见 [PipelineWorkflow](<PipelineWorkflow.kt>)，功能仍由 [PipelineFlow](<PipelineFlow.kt>)声明。只持有一个 `ConfigSelection(sectionId,nodeId?,workflowPositionId?)`，top/left 选择均由它派生；tab/status/action 是局部展示维度。空圆点只说明暂无扩展，不创建伪配置或测试。全局设置、草稿、保存冲突和独立请求凭据保持原契约，不得恢复第二份 active 状态：

| Section | 数据源 | 回答什么问题 |
|---|---|---|
| 概览 | `GET /api/stats` + `/api/plugins` | 设备在线数、活会话、11 个能力健康（degraded/failed 可点击跳配置） |
| 测试台 | —（仅 macOS 客户端） | 依赖原生麦克风/音频链路，`NavDescriptor.macOnly`：web 不显示该菜单；端到端语音测试用 macOS 客户端 |
| 设备 | `GET /api/devices` | 连过的设备、协商参数、上次会话为什么断；消息注入、个性化配置入口 |
| EchoMuse | `GET /api/echomuse/devices` + 管理台 WS `/api/admin/ws` | 批准制设备登记（在线/离线、版本、base_os）；设备卡片为「标题行（状态点+删除）+ 卡内左菜单/右面板」布局（概览 / 批准 / WiFi 配置 / 唤醒词设置 / 设备管理 / 蓝牙代理，未批准只有前两项），runtime 快照、受限音量、emOS 系统信息/固定源日志/确认重启与 BLE/GATT 工具折进卡内菜单；时间戳本地格式化；页面停留期间经管理台 WS 事件推送实时刷新（断线自动重连），无轮询 |
| 会话 | `GET /api/sessions` | 此刻活着的连接与阶段（聆听/思考/说话） |
| 对话记录 | `GET /api/conversations` | 最近聊了什么（内存环形，非持久化） |
| 用量 | `GET /api/usage` | 令牌去向、记忆/人格把上下文撑大了多少 |
| 延迟 | `GET /api/metrics` | 整轮与各段首包延迟 p50/p95、错误率（内存态） |
| 配置 | `/api/config` 等 | 全部参数 + 预设 + 每设备 overlay + 能力健康卡 + 内联连通性测试 |
| 文件 | `GET /api/files` + POST mkdir/upload/delete | 服务端存储目录浏览与管理：新建文件夹、上传模型/文件（`oww-models/` 即 EchoMuse 自定义唤醒词模型的存放处）、删除（二次确认，受保护文件拒绝） |

> ⚠️ 设备/会话/对话记录/延迟是**进程内存态**：重启清零、`online` 由会话生命周期派生、对话只留最近若干轮。长期留存请开启 `[memory]`。
> 单页模式：`?page_name=config`（支持 `&edit_device=<id>` 直接进入某设备个性化编辑）。

#### HTTP API 一览（管理动作）

| 端点 | 语义 |
|---|---|
| `GET /api/health` / `GET /api/health/engines` | 存活探针 / 11 capability 健康自检（纯只读投影，幂等无副作用） |
| `GET / POST(=PUT) /api/config` | 配置读写（部分更新：缺失保持原值、显式 `null` 清除、密钥空串保留、`expected_revision` 乐观并发 409 拦过期写入） |
| `GET/PUT/DELETE /api/devices/{id}/overlay` | 每设备白名单增量（POST = PUT、`POST .../overlay/delete` = DELETE 别名，供 Kuikly 客户端使用） |
| `GET/POST /api/config/presets`、`POST .../{name}/apply`、`DELETE/POST .../{name}/delete` | 预设：列出 / 快照当前 / 应用 / 删除 |
| `GET /api/test/capabilities`、`POST /api/test/run` | 保存态 SQLite 快照的隔离节点/串联测试，带 revision、输入边界、阶段与真实观测口径；Web/macOS 复用公共面板，不声明硬件播放成功 |
| `POST /api/test/{llm,tts,vad}` | 旧接口保留，复用隔离服务与管理鉴权；请求级覆盖不落盘，不发布生产引擎 |
| `POST /api/test/llm/models` | 拉取上游模型列表（服务端代理 OpenAI 兼容 `GET {base}/models`；请求可带 api_base/api_key 仅本次覆盖，密钥不下发客户端） |
| `GET /api/memory/status`、`POST /api/memory/{recall,maintain,clear,rag/reindex}` | 记忆状态 / 手动试召回 / 立即维护 / 清空 / RAG 索引重建 |
| `GET /api/soul/presets`、`POST /api/soul/preview` | 人格预设清单 / 最终提示词预览 |
| `POST /api/devices/{id}/inject` | 设备消息注入（文本作为用户话语走完整管线） |
| `GET /api/echomuse/devices`、`GET /api/admin/ws`、`POST .../devices/{id}/{approve,command,settings,delete}` | EchoMuse 列表/批准/WiFi 指令/唤醒词部分更新/删除；GET token 编码进查询串，POST 走 Bearer；变更通知走管理台 WS（`/wait` 长挂已删除）；旧 reboot command 不再使用 |
| `GET .../devices/{id}/runtime`、`POST .../volume` | 手动运行状态快照；缺测显示未知、离线显示历史；音量仅 approved + emos + online + speaker，level 整数 0～127，202 仅排队、刷新 runtime 核对实际上报 |
| `GET .../devices/{id}/system/{info,logs}`、`POST .../system/reboot` | 仅 approved + emos + online；固定日志源 messages/messages_last/last_kmsg_prev/supervisor，重启必须 confirm:true；响应仅确认发送，不能断言重启完成；不提供任意 shell/OTA |
| `GET .../devices/{id}/ble/{adverts,state}`、`POST .../ble/gatt` | 独立通用 BLE 面板：广播快照/槽位/连接/发现/读/确认写/手动事件；GET 带 session_id 查询、GATT 带 session_id 元数据，三类成功回包严格核对 device_id/session_id；都走 Bearer，客户端超时45s |
| `GET /api/metrics`、`GET /api/usage` | 延迟大盘 / 令牌用量（内存态） |
| `GET /api/files?path=`、`POST /api/files/{mkdir,upload,delete}` | 服务端存储目录浏览与管理：列目录（服务端目录优先排序）/ 建目录（409=同名文件占用）/ 原始字节上传（409 需显式覆盖、413 超限）/ 删除（目录递归；data.db 等受保护文件 403）；GET token 走查询串、POST 走 Bearer，错误统一 `{"error":"..."}` |

EchoMuse 实现按职责分为 [列表与配置状态](<EchoMuseState.kt>)（含卡内菜单状态机与管理台 WS 订阅 [AdminRealtime](<AdminRealtime.kt>)）、[设备卡（标题行+卡内左菜单/右面板）](<EchoMuseDeviceCard.kt>)、[页面壳体](<EchoMusePage.kt>)、[管理状态](<EchoMuseManagementState.kt>)、[管理面板](<EchoMuseManagementCard.kt>)、[鉴权与编码](<EchoMuseAccess.kt>)：
- 进入管理区不自动读设备；加载/失败/操作结果就地呈现，日志与系统输出可滚动及复制诊断内容。断线标历史；列表/详情/runtime 的 `session_id` 是当前 control 的观测身份，变化时清旧请求代际、busy、重启确认并标旧诊断为历史；runtime 仅设备号与会话都匹配时视为当前快照，双空兼容旧服务端、不将空当通配符。
- `configureAccess(token, base)` 在加载前传入；切 base 或 token 清管理目标和输出，列表/长挂/写操作通过代际丢弃旧连接回包。
- 鉴权语义（产品决策：**局域网默认可管理**）：`server.expected_token` 为空（默认）= 管理面免 token 可用（系统操作/管理接口直接放行，不再要求双 token 非空）；配置了 token 即收紧为必须鉴权——web 用 `?token=URL编码后的管理令牌` 进入，不附裸查询参数，不在诊断内容中复制 token；macOS 服务器设置提供密码型管理令牌输入，只保留在当前 Pager 会话，不落 SP/配置。设备侧看 `echomuse.token`：空 = 局域网信任设备（shell/BLE 通道不要求设备令牌），非空 = 设备必须带 `X-Em-Token`。请求凭据由 [AdminAccess](<AdminAccess.kt>)统一读取，不得复用配置草稿 expectedToken；受保护真实服务仍需独立联调。
- 唤醒设置以固件类型发送：`owwOnDevice=off|shadow|on`、`wakeSoundLevel=quiet|medium|loud`，旧 bool/number 只在读取时迁移（旧数字统一 medium）；阈值滑杆按官方量程 0.1～0.975、步进 0.025、Precise↔Eager 反向（常量与映射在 [EchoMuseWakeWord](<EchoMuseWakeWord.kt>)），轨外历史值读数保真、把手钉端点、拖动才落轨，保存仍拒非正数（服务端 `(0,1]`）；shadow 为官方诊断档、仅当前已处于该档时显示为选项，保存/重填后按当前值重建选项；提示音档位仅开关打开时显示；只发送本表单字段，服务端按部分更新合并，保留 BLE 等其他设置。唤醒词模型显示与选择规则在 [EchoMuseWakeWord](<EchoMuseWakeWord.kt>)（锚定上游默认 `hey_jarvis_v0.1`、官方四款与 display_name 规则）：存储为空即设备默认 hey jarvis 并明示，下拉列官方四款 + 服务端 `oww-models/` 已上传自定义模型 + 已存自定义值（[buildOwwOptions] 纯函数去重），保存恒发非空生效模型名（设备 config 对空 `owwModel` 不生效，恢复默认须显式发默认模型名）。卡片唤醒面板提供「管理模型文件」入口跳「文件」页 `oww-models/` 目录；**上传 ≠ 安装**——设备只认本地已装模型，未安装的模型选了也听不到唤醒词。

「文件」实现按职责分为 [纯逻辑](<FilesModel.kt>)（相对路径校验与服务端同规则镜像、清单解析安全兜底、尺寸/`yyyy-MM-dd HH:mm` 本地时间格式化——零 Kuikly 依赖，由普通 JVM 测试模块 `:adminCoreTests` 直接回归）、[目录状态](<FilesState.kt>)、[页面](<FilesPage.kt>) 与 [平台选文件桥](<UploadFilePicker.kt>)：
- 写动作只走服务端 POST 端点（`mkdir`/`upload`/`delete`，即 NetworkModule GET/POST 约束下的别名）；GET 把 token 编进查询串、POST 走 Bearer；`{"error":"..."}` 文案就地透出（409 同名不静默覆盖、403 受保护、413 超限）。
- 删除一律**行内二次确认**（先点武装「再次点击确认删除」，换目录/刷新即复位）；目录删除由服务端递归，受保护文件不给删除按钮。
- 上传平台矩阵：**web（隐藏 `<input type=file>` + fetch 流式，独立 `window.__xzAdminUploadFile` 槽位）与 macOS（复用 XiaoZhiModule 通用文档选择器 + `httpRequestBinary`，180s 超时）可用；android/ios 回调"当前平台不支持上传"**。选文件桥与 OTA 固件选包同型但完全独立（槽位/字节缓存分离，互不串包）。
- EchoMuse 设备列表加载成功后顺带静默拉取 `oww-models/` 的 `.onnx` 文件名（`modelFilesRefresh`，失败保持现状），唤醒词下拉据此追加自定义模型。

BLE 独立分为 [纯校验/codec/确认指纹](<EchoMuseBleCodec.kt>)、[请求与状态](<EchoMuseBleState.kt>)、[面板](<EchoMuseBleCard.kt>)：
- 已批准设备可展开单设备面板；在线、`ble_connect` 和非空当前会话才可请求，GATT 还要求每设备扫描/连接都开启；服务端 `ble_proxy` 与双 token 是额外门控，不能由页面开启。无 session 的旧服务端失败关闭，不复用 emOS runtime 双空兼容策略。
- 广播快照可选择 public/random MAC 或手填；服务树展示 UUID/声明 handle/value_handle/props 与描述符。特征读写必须使用 value_handle，描述符只用其 handle；HTTP200 中 result.ok=false 仍是失败，slots 按 t=slots 判断；read 必须显式字符串 value 且合法 base64，缺测不能假空值。
- hex 非空、最多512字节、只允许ASCII空白分隔的完整hex字节；转换标准base64。写入二次确认捕获设备、session、外设MAC/地址类型、handle、字节与response。任何草稿变化清确认；外设选择变更丢旧回包但不解除在途busy，编辑不可能撤销已发送操作；bridge设备/base/token/session/在线变化使旧代际失效。独立请求序号仅释放自己的busy。
- response=true成功只代表ATT接受，response=false仅代表设备发送，都不是IoT业务动作完成证明；CCCD UUID 0x2902由用户显式选择描述符handle，普通写0100/0200/0000，不猜handle、不新增subscribe协议。GET state仅手动读取有界notify/disconnected窗口，无后台高频轮询，也不承诺实时或完整历史。
- BLE设置仅POST部分更新bleProxyEnabled/bleProxyConnections，沿用旧settings接口不附session元数据（前端代际仍检查）；结果只说明已保存/在线推送排队，不假定固件已应用。广播、服务树、原始输出均有界滚动。纯函数由普通测试模块 `:bleProtocolTests:jvmTest` 直接引用同一生产 codec 验证，无 Kuikly/原生宿主依赖，且不进入生产 bundle；跨端编译不等于真实外设联调。

```bash
## 构建 web 管理页（产物含 index.html 与 nativevue2.js，由 [server].admin_dir 指向）
cd client && ./gradlew :apps:h5App:publishWeb

## 启动 server（admin_dir 默认 ../client/apps/h5App/web，按实际构建输出调整）
cd server && ~/.cargo/bin/cargo run --features sherpa -- --config-db ./data/data.db
## 浏览器访问 http://127.0.0.1:8000/（管理台）
```

---

## 上级入口

[客户端指引](<../../../../../../../AGENTS.md>)

## 本级资料

- [管理台界面设计与验收记录](<../../../../../../../../docs/designs/admin-ui-design.md>)：信息架构、上下文页面、健康与记忆验收的历史记录。
