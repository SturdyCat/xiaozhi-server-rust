# 公共管理台维护

这里是跨端管理页面、表单和网络状态；macOS 专用测试台不属于 commonMain。

## UI 与布局

- [AdminTheme](<AdminTheme.kt>)提供 token，现有[组件](<AdminComponents.kt>)、[表单控件](<AdminFormControls.kt>)、[widgets](<AdminWidgets.kt>)统一封装官方控件；不要绕过封装散写样式。
- 页面/配置分区由[AdminNav](<AdminNav.kt>)和[ConfigSections](<ConfigSections.kt>)常量表驱动，id 稳定、order 确定。新增分区同时补渲染分支。
- Tabs/PageList 显式提供 pageItemWidth/Height；滚动事件回传 Tabs.scrollParams，点击 tab 调 scrollToPageIndex。不自画指示器。
- 嵌套 DSL 先 `val ctx=this` 再显式访问 Pager；复用渲染为文件级 ViewContainer 扩展，ViewBuilder 显式传接收者。observable 在 attr/vif 闭包内读取才被跟踪。
- 官方 compose Button 负责居中；不塞子节点 loading 菊花。NMAllResponse.data 非空，不复制恒假的 data==null 判断。

## 表单与网络契约

- [ConfigFormState](<ConfigFormState.kt>)保存草稿，[fill](<ConfigFormFill.kt>)映射读取；密钥只读 presence、写入 putSecret，空草稿不发送。新增密钥与服务端 SECRET_PATHS 同批改。
- hot/字段提示由[FormSchemaState](<FormSchemaState.kt>)消费服务端 schema，不手写另一套档位。schema 不是自动布局，engine/preset 可选项仍需两端同步。
- 409 保留草稿，显式“重新加载”，不静默覆盖。preset 应用复用服务端配置管线；overlay 编辑只提交设备增量，不能混成全局整份配置。
- NetworkModule 只支持 GET/POST，使用服务端 POST 别名；curl 的 PUT/DELETE 能用不等于页面可用。
- 人格预览 effective 全段 fill，客户端不复制预设合并。能力 enabled 与 phase 分开，异常/降级须显示原因与修复指引。
- 设备/会话/最近对话/延迟注明进程内口径，online 非心跳，不呈现为完整历史。

本级说明 · [客户端构建](<../../../../../../../AGENTS.md>) · [管理台界面设计与验收记录](<../../../../../../../../docs/admin-ui-design.md>)


---

## 管理台页面与操作

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

### 5. 管理台（Web + macOS）

管理台是 **Kuikly 多端工程**（`client/`），逻辑在 `client/shared/commonMain` 跨端复用；web 构建产物由 server 在 `/` 静态托管。

默认落地页 **管理台**（`@Page("admin")`）：一级纵向侧边栏；「配置」section 内为三级导航——二级横向 tabs（总览/预设/三分组）→ 三级纵向子菜单 + 右侧内容（布局见 `ConfigTabs.kt`，分区常量见 `ConfigSections.kt`）：

| Section | 数据源 | 回答什么问题 |
|---|---|---|
| 概览 | `GET /api/stats` + `/api/plugins` | 设备在线数、活会话、11 个能力健康（degraded/failed 可点击跳配置） |
| 测试台 | —（Web 占位） | Web 无麦克风 → 不可用说明 + 替代路径；端到端语音测试用 macOS 测试台 |
| 设备 | `GET /api/devices` | 连过的设备、协商参数、上次会话为什么断；消息注入、个性化配置入口 |
| EchoMuse | `GET /api/echomuse/devices` | EchoMuse 设备批准制登记（在线/离线、版本、base_os）；批准、重启、下发 WiFi 配置、唤醒词设置（离线可保存，上线自动推送） |
| 会话 | `GET /api/sessions` | 此刻活着的连接与阶段（聆听/思考/说话） |
| 对话记录 | `GET /api/conversations` | 最近聊了什么（内存环形，非持久化） |
| 用量 | `GET /api/usage` | 令牌去向、记忆/人格把上下文撑大了多少 |
| 延迟 | `GET /api/metrics` | 整轮与各段首包延迟 p50/p95、错误率（内存态） |
| 配置 | `/api/config` 等 | 全部参数 + 预设 + 每设备 overlay + 能力健康卡 + 内联连通性测试 |

> ⚠️ 设备/会话/对话记录/延迟是**进程内存态**：重启清零、`online` 由会话生命周期派生、对话只留最近若干轮。长期留存请开启 `[memory]`。
> 单页模式：`?page_name=config`（支持 `&edit_device=<id>` 直接进入某设备个性化编辑）。

#### HTTP API 一览（管理动作）

| 端点 | 语义 |
|---|---|
| `GET /api/health` / `GET /api/health/engines` | 存活探针 / 11 capability 健康自检（纯只读投影，幂等无副作用） |
| `GET / POST(=PUT) /api/config` | 配置读写（部分更新：缺失保持原值、显式 `null` 清除、密钥空串保留、`expected_revision` 乐观并发 409 拦过期写入） |
| `GET/PUT/DELETE /api/devices/{id}/overlay` | 每设备白名单增量（POST = PUT、`POST .../overlay/delete` = DELETE 别名，供 Kuikly 客户端使用） |
| `GET/POST /api/config/presets`、`POST .../{name}/apply`、`DELETE/POST .../{name}/delete` | 预设：列出 / 快照当前 / 应用 / 删除 |
| `POST /api/test/{llm,tts,vad}` | 内联连通性与首包延迟测试（按磁盘最新配置 ad-hoc 建引擎，无需麦克风，Web 可用） |
| `POST /api/test/llm/models` | 拉取上游模型列表（服务端代理 OpenAI 兼容 `GET {base}/models`；请求可带 api_base/api_key 仅本次覆盖，密钥不下发客户端） |
| `GET /api/memory/status`、`POST /api/memory/{recall,maintain,clear,rag/reindex}` | 记忆状态 / 手动试召回 / 立即维护 / 清空 / RAG 索引重建 |
| `GET /api/soul/presets`、`POST /api/soul/preview` | 人格预设清单 / 最终提示词预览 |
| `POST /api/devices/{id}/inject` | 设备消息注入（文本作为用户话语走完整管线） |
| `GET /api/echomuse/devices`、`POST /api/echomuse/devices/{id}/{approve,command,settings}` | EchoMuse 设备：列表 / 批准（body `label`）/ 下发指令（`reboot`、`wifi`，202 已排队、409 设备离线）/ 唤醒词设置（白名单部分更新，在线 202 `delivered:true`、离线 200 已保存） |
| `GET /api/metrics`、`GET /api/usage` | 延迟大盘 / 令牌用量（内存态） |

```bash
## 构建 web 管理页（产物含 index.html 与 nativevue2.js，由 [server].admin_dir 指向）
cd client && ./gradlew :apps:h5App:publishWeb

## 启动 server（admin_dir 默认 ../client/apps/h5App/web，按实际构建输出调整）
cd server && ~/.cargo/bin/cargo run --features sherpa -- --config config.toml
## 浏览器访问 http://127.0.0.1:8000/（管理台）
```

---

## 上级入口

[客户端指引](<../../../../../../../AGENTS.md>)

## 本级资料

- [管理台界面设计与验收记录](<../../../../../../../../docs/admin-ui-design.md>)：信息架构、上下文页面、健康与记忆验收的历史记录。
