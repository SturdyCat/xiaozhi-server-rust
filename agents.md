# agents.md — 给 AI 编码助手的项目指引

> 本文件供 CodeBuddy / Claude Code / Cursor / Codex 等 AI 编码助手在本仓库工作时阅读。
> 它聚焦**本仓库特有的环境坑与协议约束**，通用 Rust 知识不在此赘述。

> 📌 模块级职责与复杂逻辑以 `server/src/*.rs` 顶部的 `//!` 模块注释为**权威说明**（配置加载优先级、`/api/config` 读写语义、引擎线程预算、协议二进制封装、四大引擎的 `spawn_blocking` 隔离等均已下沉到源码）。本文件聚焦 AI 易踩的**陷阱速查**；陷阱涉及的具体实现以源码 `//!` 注释为准，下文各条已加交叉链接。

## 1. 项目是什么

`xiaozhi-server-rust` 是一个用 Rust 实现的 **小智（xiaozhi-esp32）语音终端服务端**：

- 本地 ASR：SenseVoice INT8（离线，`sherpa-onnx`）
- 本地 TTS：Kokoro INT8（离线，`sherpa-onnx`）
- VAD：Silero VAD
- 远程 LLM：OpenAI 兼容的 `chat/completions` HTTP 接口
- 传输：WebSocket，协议**严格对齐本地 `xiaozhi-esp32` 固件**（文本消息 + 二进制 Opus 音频帧，支持 v1/v2/v3 二进制协议版本）

设计目标：在 Intel Celeron N5105（x86-64）这类低功耗主机上用 Docker 跑一个 ASR/TTS 离线的小智服务端。

权威协议来源（相对路径，与固件仓库并列时）：

- `../xiaozhi-esp32/docs/websocket_zh.md`
- `../xiaozhi-esp32/main/protocols/websocket_protocol.cc`

## 2. 技术栈与关键依赖

| 依赖 | 版本 | 说明 |
|------|------|------|
| `sherpa-onnx` | **1.13.8** | 本地 ASR/TTS；`optional`，仅 `sherpa` feature 引入 |
| `axum` | 0.8 | 内置 `ws`，**不**用 `tokio-tungstenite` |
| `tokio` | 1.44 | `features = ["full"]` |
| `reqwest` | 0.12 | `default-features = false, features = ["json", "rustls-tls"]` |
| `audiopus` | 0.2 | Opus 编解码（`sherpa` feature 下启用，需系统 `libopus`） |
| `rubato` | 0.15 | 重采样（`sherpa` feature 下启用） |
| `serde` / `serde_json` / `toml` / `anyhow` / `thiserror` / `tracing` / `uuid` | 见 `Cargo.toml` | — |

> `sherpa-onnx` 版本务必锁定 **1.13.8**（最新稳定版，2026-09 发布）。最初 spec 写的 `0.1` 是错的。API 已对照 1.13.8 rustdoc 核对：`OfflineSenseVoiceModelConfig`、`OfflineTtsKokoroModelConfig`、`VoiceActivityDetector` 字段与方法签名与实现一致。

## 3. 构建与测试（⚠️ 环境相关命令）

本机的 `cargo` 不在 PATH，且全局 cargo 配置把 USTC 镜像指向 git 协议（返回 404）。**所有 cargo 命令必须**：

```bash
# 1) 用本地 cargo（PATH 未预置）
export PATH="$HOME/.cargo/bin:$PATH"      # 或直接使用绝对路径 ~/.cargo/bin/cargo

# 2) 每次 cargo 调用追加 USTC sparse 镜像覆盖，否则报 404：
--config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'
```

完整示例：

```bash
# 编译检查（无 feature，不下载原生库；运行必须 --features sherpa）
~/.cargo/bin/cargo build \
  --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'

# 类型检查 sherpa feature 代码（DOCS_RS=1 跳过原生库下载，见「验证不下载模型」）
DOCS_RS=1 ~/.cargo/bin/cargo check --features sherpa \
  --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'

# 单元测试
~/.cargo/bin/cargo test \
  --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'

# 真实引擎（需联网下载 sherpa 原生库 + 系统 libopus + 模型文件）
cd server && ~/.cargo/bin/cargo run --features sherpa -- --config config.toml
```

## 4. Feature 门控（重要）

`Cargo.toml` 中：

- `default = []`：无真实引擎，仅供 `cargo check/test` 编译（运行会报错）。
- `sherpa = ["dep:sherpa-onnx", "dep:audiopus", "dep:rubato"]`：真实引擎（**无 mock**：ASR=SenseVoice、TTS=Kokoro、LLM=HTTP、VAD=Silero）。

> 无 `backend` 配置项（已随 mock 移除；旧 config.toml 里的 `backend` 键会被 serde 静默忽略，管理页保存一次即写成新 schema）。默认（无配置文件）模型路径即 `/models/...` 生产值；`[llm]` 需填 `api_base`/`api_key`。ESP 接入走完整正式流水线；macApp 测试台 hello 带 `test:true`，走 `asr_test/tts_test/llm_test` 三个独立服务端点（非测试会话发送这三类消息会被忽略）。

## 5. 🚨 关键约束与陷阱（AI 最容易踩）

### 5.1 协议 `type` 标签必须是**小写**

`xiaozhi-esp32` 固件发送/期望**小写** `type` 标签：`hello` / `listen` / `abort` / `mcp`（设备→服务器）与 `hello` / `stt` / `llm` / `tts` / `system` / `custom` / `mcp`（服务器→设备）。

`serde` 的 `#[serde(tag = "type")]` 默认用 Rust 变体**大写**名（`Hello`/`Stt`/`Llm`…）做匹配，会在运行时直接报：

```
unknown variant `hello`, expected one of `Hello`, `Listen`, `Abort`, `Mcp`
```

**必须**加：

```rust
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMessage { Hello(ClientHello), Listen { .. }, Abort { .. }, Mcp { .. } }

#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMessage { Hello { .. }, Stt { .. }, Llm { .. }, Tts { .. }, System { .. }, Custom { .. }, Mcp { .. } }
```

> 这一 bug **`cargo check` 完全无法发现**，只有真实 WebSocket 客户端（`tests/mock_client.py`）才能暴露。任何修改协议枚举的 PR 都必须重跑协议联调。

> 协议枚举定义与 serde 约束的权威说明见 `server/src/protocol.rs` 模块注释。

### 5.2 LLM 恒为真实 HTTP（无 mock）

LLM 只有 OpenAI 兼容 Responses API 一条路径；联调失败先检查 `[llm].api_base`/`api_key`/`model` 配置（服务端启动即构建引擎，模型/配置缺失会给出明确报错）。

### 5.3 二进制协议版本 = 设备 hello 的 `version`

- `version` 字段 = **二进制协议版本（1/2/3）**，不是握手协议号。
- 服务器**下行**二进制帧必须使用与设备相同的版本（`wrap_downlink`）。
- 上行按版本剥离头部（`unwrap_uplink`）。v1 裸 Opus；v2 16 字节头；v3 4 字节头。
- 建议先用 v1 真机验证，再切 v2/v3。

> 二进制帧字节布局（v1/v2/v3）的权威表见 `server/src/protocol.rs` 模块注释。

### 5.4 服务器 hello 的 `audio_params` = 下行解码参数

设备读取服务器 hello 的 `audio_params.sample_rate` / `frame_duration` 来解码下行 TTS 音频。上行仍按设备自己的 16k。下行采样率由 `audio.downlink_sample_rate`（默认 24000）决定。

> 上行/下行协商逻辑见 `server/src/ws.rs` 的 `handle_handshake`。

### 5.5 构建镜像的 Rust 版本必须与生成 `Cargo.lock` 的 cargo 对齐（当前 1.90）

`Cargo.lock` 由本机 cargo **1.90** 生成，锁定的传递依赖需要较新 Rust：`idna_adapter`（`reqwest → url → idna`）要求 `edition2024`（Cargo ≥ 1.85 才能解析）；`time` / `icu_*` 要求 `rustc 1.88`。`Dockerfile` 构建阶段用 **`rust:1.90-bookworm`**（与本机 cargo 同版本，保证 lock 一定可编译）。构建镜像版本约束见根 `Dockerfile`。

**不要降级镜像 Rust**；也不要在无 lock 限定的情况下 `cargo update` 导致依赖需要更高 Rust。若升级了本机 cargo，记得同步抬高此处的镜像版本。

### 5.6 本地（macOS）编 `--features sherpa` 需要 Homebrew 的 `opus`（绕开 autoreconf）

`audiopus_sys` 优先用 `pkg-config` 找系统的 `libopus`；找不到就回退去**从自带 Opus 源码 autotools 编译**，而本机只装了 `autoconf`/`autoreconf`/`glibtoolize`，**缺 `automake`（无 `aclocal`）**，于是 `autoreconf` 直接失败：

```
Can't exec "aclocal": No such file or directory
autoreconf: error: aclocal failed with exit status: 2
```

**最省事的修法（macOS 本地验证用）**：

```bash
# 1) 把 Homebrew 工具链与 pkg-config 路径加入 PATH（非交互 shell 默认不带 /opt/homebrew/bin）
export PATH="/opt/homebrew/bin:$PATH"
# 2) 装 opus，让 audiopus_sys 走 pkg-config 动态链接，彻底跳过源码编译
brew install opus
export PKG_CONFIG_PATH="/opt/homebrew/lib/pkgconfig:$PKG_CONFIG_PATH"
# 3) 此时再编 sherpa 特性即可（动态链到 /opt/homebrew/lib/libopus.dylib）
~/.cargo/bin/cargo check --features sherpa \
  --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'
```

- 注意：这样编出的二进制**动态依赖 Homebrew 的 opus**，仅用于本地验证代码能否编译，**与 Docker 部署无关**（Docker 走 Debian `libopus0` + 静态/系统链接，CI 已验证可过 `audopus_sys`）。
- 若想严格复现源码编译路径，则需再 `brew install automake libtool`（并把 `libtoolize` 链到 `glibtoolize`），但 pkg-config 路径更省事，推荐。

### 5.7 容器启动会自动检测并下载缺失模型（`docker-entrypoint.sh`）

报错 `tokens.txt does not exist` / `创建 SenseVoice 识别器失败` 的根因是 `/models` 里没有模型文件。Docker 入口脚本在 `exec` 服务器**之前**会自检关键文件（`silero_vad.onnx`、`SenseVoiceSmall/model.int8.onnx`、`Kokoro/model.int8.onnx` 等，**官方包内文件名是 `model.int8.onnx`，不是 `model.onnx`**）：

- **缺失 → 自动从 k2-fsa/sherpa-onnx 官方 GitHub Release 下载整包 tar.bz2 并解压**到挂载的 `/models`（默认行为），下载后持久化，后续启动检测到即跳过（按模型粒度幂等 + .part 断点续传）。
- 默认**直连原始地址**；docker compose 配置了 `GITHUB_PROXY` 才走代理（如 `https://tvv.tw/`）。`SENSEVOICE_URL` / `KOKORO_URL` / `SILERO_VAD_URL` 可覆盖为完整直链（内网镜像，不会被二次套代理）。
- curl 带 `--connect-timeout 15 --retry 3`（避免连接假死 134s）；下载/解压失败会让入口**中止启动**（未设 `XIAOZHI_ALLOW_MISSING_MODELS` 时），避免带着缺模型崩溃重启循环。
- 行为开关 `XIAOZHI_AUTO_DOWNLOAD_MODELS`：`missing`（默认）/`force`（每次重下）/`off`（不下载）。
- 运行期镜像需装 `curl` + `bzip2`（`tar` 自带）用于下载与解包；`docker-compose.yml` 的 `./models` 挂载**必须可写**（不要 `:ro`）。
- 离线/内网：先 `./server/scripts/download_models.sh /host/models` 预置再挂载，或设 `XIAOZHI_AUTO_DOWNLOAD_MODELS=off`。

默认模型包（已完整下载验证，与 config.example.toml 路径一一对应）：`sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09`（SenseVoice INT8，`model.int8.onnx`）、`kokoro-int8-multi-lang-v1_1`（Kokoro INT8 **中英双语**，含 model.int8.onnx/voices.bin/tokens.txt/espeak-ng-data/lexicon-zh.txt/lexicon-us-en.txt/dict（jieba）/date-zh.fst）、`silero_vad.onnx`（Silero VAD）。⚠️ 此前误判 `kokoro-int8-multi-lang-v1_1` 不是完整包（流式列清单被管道截断所致），实际 144MB/417 文件完整；`kokoro-int8-en-v0_19` 仅英文、无 lexicon，中文场景勿用。

## 6. 项目结构速查（monorepo）

本仓库为 monorepo：`server/` 是 Rust 服务端，`client/` 是 Kuikly 多端工程（管理后台 web）。

```
server/                        # Rust 服务端
  src/
    main.rs       入口：加载配置 → 初始化引擎 → 启动 Axum
    config.rs     TOML 配置（默认值=生产路径；含 GET/POST /api/config 读写）
    protocol.rs   消息枚举 + 二进制版本封装（v1/v2/v3）—— 改这里必看 §5.1
    ws.rs         WebSocket 网关：握手 / 协商 / 鉴权 + 静态托管管理页（/）
    session.rs    每连接会话状态机 + 语音流水线（支持 abort）
    asr.rs / vad.rs / tts.rs / llm.rs / engine.rs / audio/
  config.toml / config.example.toml
  scripts/download_models.sh
  tests/mock_client.py         零依赖 WebSocket 联调客户端
client/                        # Kuikly 多端工程（管理后台 web + macOS 测试台）
  settings.gradle.kts         rootProject.name = "client"
  gradle.properties           kuiklyVersion 唯一来源（升级需与根 build.gradle.kts 的 buildscript 双写同步）
  build.gradle.kts            Kuikly 插件经 buildscript classpath 引入 + 插件版本锁定（含 KSP）
  kotlin-js-store/yarn.lock   Kotlin/JS npm 依赖锁（⚠️ 必须入库，缺失则 yarn 解析漂移）
  shared/
    commonMain/               公共代码：@Page("config") 管理后台（跨端）+ 配置表单 + NetworkModule
    macosArm64Main/           仅 macOS(arm64) 编译：@Page("test") ASR/TTS 测试台 + XiaoZhiModule
    （js(IR) 目标在此模块：@Page 注册靠 KSP(core-ksp)，业务包由 Kuikly 插件打包）
  apps/
    h5App/                    Web(H5) 壳：渲染器(core-render-web:h5) + Main.kt；publishWeb 汇聚 web/
    web/                      产物契约：index.html(入库) + nativevue2.js(业务包) + h5App.js(壳)
    androidApp/ iosApp/ ohosApp/   各原生宿主
    macosApp/                 macOS 宿主（原生，Mac Catalyst）：ASR/TTS 测试台，复用 iOS 渲染器
```

> server 在 `/` 静态托管 `client/apps/h5App/web`（`[server].admin_dir` 默认值即此；
> 容器内由镜像内置 `XIAOZHI_ADMIN_DIR=/app/web` 覆盖），并通过 `GET/POST /api/config`
> 让管理页读写 `config.toml`。本地产出管理页：`cd client && ./gradlew :apps:h5App:publishWeb`。

### 5.8 client（Kuikly）构建链要点与陷阱

- **web 双 bundle 架构**（对齐 xiaoya-player）：`shared` 持有 js 目标 → Kuikly 插件 `packLocalJSBundleRelease` 产业务包 `nativevue2.js`；`apps/h5App` 是壳（渲染器 + 入口）→ webpack 产 `h5App.js`；`web/index.html` 先载业务包、后载壳。**js 消费方无法解析无 js 目标的 KMP 模块**——不要把 js 目标从 shared 挪走。
- **Web 渲染器真实坐标是 `com.tencent.kuikly-open.core-render-web:h5`**（artifactId 为 `h5`）。`com.tencent.kuikly-open:core-render-web` 在腾讯镜像上 404（实测 Could not resolve）。
- **DSL 陷阱**：容器带 `@ScopeMarker`（@DslMarker）——body 的嵌套容器里不能隐式访问 Pager 成员；官方口径 `val ctx = this` + `ctx.xxx`。可复用 UI 片段写成**文件级** `private fun ViewContainer<*,*>.xxx()` 扩展（类内成员扩展实测编译失败）。持有 `ViewBuilder` 值在嵌套闭包内执行必须显式传接收者（`page.content(this)`），隐式 `page.content()` 报 `No value passed for parameter 'p1'`。
- **UI 必须用 Kuikly 官方组件与布局（用户多次强调，勿自造样式替代官方机制）**：写 UI 前先查 `.agents/skills/kuikly-*`（kuikly-ui-framework 内含官方文档/源码克隆 `references/KuiklyUI/`）与官方组件清单；编码规范见 `.codebuddy/rules/kuiklyDSL.mdc`。已整改：下拉=官方 AlertDialog、开关=官方 Switch、标签页=官方 **Tabs+PageList**（TabItem 官方结构 + `indicatorInTabItem` 官方指示条，不自画选中胶囊）、按钮=官方 compose Button、backend 选择=官方 AlertDialog 下拉（手搓 segmentedControl 已删——官方 SegmentedControlIOS 是 iOS 渲染器专属、web 无实现）、多行散文=官方 TextArea（`labeledTextArea`）。**PageList 必须显式设 `pageItemWidth`/`pageItemHeight`**（官方示例按导航/tab 高度手算；AdminShell/ConfigPage 各自算好传入 `tabbedPanel`），不设则 item 无尺寸约束整页塌成单行（实测）。Tabs↔PageList 联动 = PageList `scroll` 事件回传 ScrollParams 喂 `Tabs.scrollParams`、点击 tab `scrollToPageIndex(index)` 翻页；封装见 AdminTheme.kt `tabbedPanel`。
- **组件官方化审计结论（2026-10-05 全量）**：AdminTheme.kt 是唯一组件层，全部基于官方组件组合（Text/View flex/官方 Button/Input/TextArea/Switch/AlertDialog/Tabs/PageList/Scroller/ActivityIndicator）；无官方对应物的组合件仅剩：`statusBadge`（官方无徽标）、`waveformPlayer`（官方无音频波形/播放器 UI）、`sidebarItem`+`largeTitleBar`+`groupedCard`+`ToastHost`（官方无侧边栏/导航/卡片/Toast 组件，属 View+Text 官方布局组合）。新增 UI 一律先查官方清单，禁止绕过 AdminTheme 直接散写。
- **API 位置**：`Color`/`Border`/`BorderStyle` 在 `com.tencent.kuikly.core.base`（不在 base.attr）；**按钮用官方 compose Button**（`com.tencent.kuikly.core.views.compose.Button`：`titleAttr` 设文字/颜色且须写在 attr 块内、`highlightBackgroundColor` 按压高亮；文字居中由 ButtonView 内部 `justifyContentCenter+alignItemsCenter` 保证——手搓 View+Text 按钮（allCenter/textAlignCenter/flex）在 Catalyst 上居中实测不可靠，「连接/断开」多次偏侧即此因；官方 Button 无子节点插槽，loading 态用灰底+文案切换，不塞菊花）；无 `paddingHorizontal/Vertical`（用 padding(left/right/top/bottom)）；Input 文本变化事件是 `event { textDidChange { } }`。
- **Docker 构建内存**（ACR 构建机实测 OOM 表现：日志戛然而止 + rpc EOF）：web 阶段 Gradle/Kotlin daemon/webpack-Node 三处显式限堆（1280m/1024m/1024m，取 xiaoya 实测口径）+ `-PwebSourceMap=false` 关 source map；⛔ 不要调高。
- **settings.gradle.kts 勿设 PREFER_SETTINGS**（会丢 Kotlin/JS 插件自动加的 nodejs.org dist 仓库，org.nodejs:node 必然解析失败）；根工程/子模块勿声明项目级 repositories（多余仓库的 DNS 异常会中断整条解析链）。

### 5.9 macosApp（Mac Catalyst）构建与运行要点

- **只能用「My Mac (Mac Catalyst)」运行目标**：渲染器 OpenKuiklyIOSRender 是 UIKit 的；选原生 "My Mac" 必报 `'UIKit/UIKit.h' file not found`。防呆已内建：target 的 preBuildScripts 守卫检查 `EFFECTIVE_PLATFORM_NAME != -maccatalyst` 时直接报中文指引（原生目标构建在编译前即被拦下，双向实测）。
- **生成顺序**：`xcodegen generate` → `pod install`（xcodegen 重写 .xcodeproj 会抹掉 Pods 集成，重新生成后必须重跑 pod install）。开 `macosApp.xcworkspace`。
- **签名**：project.yml 用 Manual + ad-hoc（`CODE_SIGN_IDENTITY: "-"`，即 Xcode 的 Sign to Run Locally）；Automatic 无开发者团队直接拒建。⚠️ 曾试过「iOS 平台 + SUPPORTS_MACCATALYST」做 Catalyst-only 杜绝误选：iOS 平台目标强制要求 development team，而本机全部 Apple Development 证书均无 Mac 开发能力（3 个团队逐一实测均报 `No "Mac Development" signing certificate matching team ID`）——勿回退那套，守卫脚本已解决误选问题。
- **Podfile Catalyst 配方**：`platform :ios`（**不能** `:osx`——UIKit Pod 按 macOS 原生平台编不出产物）+ post_install 给全部 Pod 目标开 `SUPPORTS_MACCATALYST`、排除 x86_64、**强制 `IPHONEOS_DEPLOYMENT_TARGET=15.0`**（⚠️ Catalyst 的 macOS 部署目标由 iOS 目标**映射**而来：iOS 13→macOS 10.15 会被新 Xcode SDK 拒建；只改 MACOSX_DEPLOYMENT_TARGET 对 iOS 平台 Pod 无效，实测踩坑）。
- **框架路径**：macosApp 在 `apps/` 子目录，shared 在 `client/` 根——相对路径要上**两级**（`../../shared/...`），且链接搜索路径用 `$(PROJECT_DIR)/../../shared/...`（纯相对路径 ld 找不到）；产物目录是 `releaseFramework`（无 releaseShared）。
- **macabi 平台补丁（已自动化）**：Kotlin/Native 无 macabi 目标，静态框架（ar 归档）内目标文件平台标记是 macOS，ld 拒绝链入 Catalyst。`shared/linkReleaseFrameworkMacosArm64` 等框架链接任务已 `finalizedBy` 补丁任务，自动执行 `scripts/patch_framework_macabi.py`（ar x → 改写 LC_BUILD_VERSION platform 1→6、minos≥12.0 → ar cr → ranlib，幂等）。本机 vtool 不认 macabi 平台名，勿走 vtool。

## 7. 运行与联调

```bash
# 启动服务（后台；需 --features sherpa 与本地模型）
~/.cargo/bin/cargo run --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"' &
# 健康检查
curl http://127.0.0.1:8000/api/health   # => xiaozhi-server-rust ok

# 协议联调（零第三方依赖，纯标准库）
python3 server/tests/mock_client.py
# 期望输出含：握手成功 → 服务器 hello（downlink sr=24000）→
# 文本消息序列 ['stt','llm','tts:start','tts:sentence_start','tts:stop']
# → 收到若干下行二进制帧 → PASS
# 可选参数：--host 127.0.0.1 --port 8000 --token <bearer>（expected_token 非空时）
```

联调客户端断言顺序：**hello → listen start → stt → llm → tts:start → tts:sentence_start → 二进制帧 → tts:stop**。任一缺失即 FAIL。

## 8. 配置加载优先级

`XIAOZHI_CONFIG` 环境变量 → `--config <path>` 参数 → 内置默认（/models 生产路径）。

## 9. 已知限制 / 未实现

- **激活流程（OTA/activate）本期未实现**：接受任意设备，`expected_token` 为空则跳过鉴权。
- **VAD 模型加载**：每次会话 `VoiceActivityDetector::create` 会加载模型；连接数大时建议池化（待优化）。
- **ASR 流式**：SenseVoice 为离线逐段识别；如需逐字流式可后续换 Zipformer `OnlineRecognizer`。
- **未启用 `sherpa` 的编译下行音频为空帧**：`encode_opus_frame` 占位实现返回空，仅供编译/测试。
- 残留编译 warning（预留未用字段/变体），不影响功能。

## 10. 提交规范（重要）

本项目约定：**不要自动提交代码**。完成改动并编译通过后停在工作区，由用户决定何时、以什么范围提交。不要自行 `git add` / `git commit`。

## 11. 更多上下文

- 设计取舍、分阶段实施、协议对齐要点见 [`PLAN.md`](./PLAN.md)。
- 用户向文档、快速开始、Docker、协议要点见 [`README.md`](./README.md)。
