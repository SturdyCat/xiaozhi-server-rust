# Docker 部署与模型准备

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

## 9. Docker 部署细节

### 底座发布镜像

发布镜像由根目录 [`Dockerfile`](<../Dockerfile>) 构建，Rust 服务端使用
`sherpa,nlu,wasm-plugins` features：

- `sherpa` 提供真实 ASR/TTS/VAD 模型与音频运行时；
- `wasm-plugins` 编译 WASM Component 宿主与 WIT 契约 `xiaozhi:plugin@0.2.0`；
- 默认构建保持发布进程为底座装配。

WASM 宿主随 `/app/server` 发布。**官方系统插件**由 [`tools/build-system-plugins.sh`](<../tools/build-system-plugins.sh>) 构建到 `build/system-plugins/`（15 个 `.wasm`，含 `hello.official` 探针），随镜像置于 `/app/system-plugins`（可用 `XIAOZHI_SYSTEM_PLUGINS_DIR` 覆盖）；boot 扫描后恢复注册与激活状态。**用户插件**仍走管理界面 `POST /api/plugins/upload/raw` 上传，运行期 artifact 落在数据卷。

⛔ **契约升级运维（实测踩过）**：`wasm_boot.rs::import_system_plugins_from` 只在数据卷**尚无同 id 产物**时导入系统插件，刻意不覆盖用户升级/停用状态。因此把宿主从 0.1 升到 0.2 后，数据卷里的旧契约产物不会被替换，装载校验失败会让插件列表变空。必须重建产物并逐个重走 `upload/raw → register → activate`；同版本号重传会被 409「版本不升级」拒绝，必要时升版本号。验收口径：`GET /api/plugins` 返回 15 插件 / 14 Active / 14 扩展点 resolved，`hello.official` 保持 Registered 不激活（它声明 13 个扩展点且 official=true，激活会造成双官方冲突）。完整纪律见 [plugins/wasm/AGENTS.md](<../plugins/wasm/AGENTS.md>)。

NLU 训练沿用 compose 显式挂载的宿主 Docker socket，训练脚本作为服务端编译期源码输入保留。

### 模型自动下载

容器入口（`docker-entrypoint.sh`）启动时检测关键模型文件（`silero_vad.onnx`、`SenseVoiceSmall/tokens.txt`、`Kokoro/model.int8.onnx`）：

- **缺失则自动下载**到挂载的 `/data/models`（从官方 GitHub Release 拉整包 tar.bz2 本地解压，幂等 + 断点续传）。
- 默认直连原始地址；compose 配置 `GITHUB_PROXY` 才走代理。下载地址可用 `SENSEVOICE_URL` / `KOKORO_URL` / `SILERO_VAD_URL` 覆盖为完整直链（内网镜像）。
- 行为开关 `XIAOZHI_AUTO_DOWNLOAD_MODELS`：`missing`（默认）/ `force` / `off`。
- 入口只准备共享的模型/音频运行时文件；不拉取插件 artifact，也不执行训练任务。

> 不手动预置模型也能直接 `docker compose up`。离线/内网环境：先 `SENSEVOICE_URL=<内网镜像> ./server/scripts/download_models.sh /host/data/models` 预置再挂载，或设 `XIAOZHI_AUTO_DOWNLOAD_MODELS=off`。

### 部署机指令集

构建期固定 `RUSTFLAGS="-C target-cpu=x86-64-v2"`（见 `Dockerfile`），兼容 N5105（Tremont，无 AVX）。**不要用 `native`**——会在构建机嵌入部署机不支持的 AVX 指令，运行时 `SIGILL` 崩溃。

### 真实模型/音频与 NLU 运行时（`sherpa,nlu,wasm-plugins`）本地依赖

1. 系统 `libopus`；2. 网络（构建期原生库下载）；3. 本地模型文件：

```bash
./server/scripts/download_models.sh /host/data/models     # 裸机部署时手动下载
cd server && ~/.cargo/bin/cargo run --release --features sherpa,nlu,wasm-plugins -- --config-db /host/data/data.db
```

## 构建与故障边界

- [Dockerfile](<../Dockerfile>)必须以仓库根为上下文；Web 阶段严格保留源码/产物断言、Gradle/Kotlin/Node 内存上限（ARG 化，**默认值 = ACR 4-8GB 构建机校准档**，勿调高默认）与关闭 source map 的生产配置。[GHA 手动构建](<../.github/workflows/docker-amd64.yml>)按 16GB/4vCPU 档经 build-args 放宽档位，ACR 自动构建不传参维持原档，两边互不影响。
- Web 基础镜像已预装与 Wrapper 相同版本的 Gradle，Docker 内直接调用 `gradle`，不再执行 `./gradlew` 重复下载发行包；首条配置任务核对基础镜像内 Gradle 与 [Wrapper 配置](<../client/gradle/wrapper/gradle-wrapper.properties>)的版本，不一致直接失败。升级时两处同步；本地开发继续使用 Wrapper。`Downloading ...gradle-*-all.zip` 后出现 `Read timed out (10000ms)` 是发行包下载失败、尚未编译业务代码，不是 NLU 编译错误或 OOM。此修复只消除重复发行包下载，Maven/npm 等依赖仍需网络或有效缓存。
- 镜像构建跨次缓存两层：buildx `type=gha,mode=max` 层缓存（只改 server 不重编 web，反之亦然）+ `RUN --mount=type=cache` 内容缓存（Gradle 主目录 / cargo registry+git，清单失效也不重下载）。首次构建冷建，第二次起生效；GHA 仓库缓存配额 10GB LRU，超限只是退化冷建。产物 `target` 与 dummy 依赖层设计不挂缓存，进层才成立——详见 Dockerfile 头部 ♻️ 段。
- 镜像发布两条来源**互斥**：阿里云 ACR「源代码自动构建」（push 即建 amd64 `latest`）与 [GitHub Actions 手动构建](<../.github/workflows/docker-amd64.yml>)（仅 linux/amd64，推 `registry.cn-hangzhou.aliyuncs.com/notip-com-cn/xiaozhi-server-rust`，需仓库 secrets `ACR_USERNAME`/`ACR_PASSWORD`）。同一 tag 只能由其中一处产出——启用 GHA 产出 `latest` 时先到 ACR 控制台关闭该仓库的自动构建规则（或改其输出 tag）。
- Rust 镜像版本与[锁文件](<../server/Cargo.lock>)同步，不随意降级；[构建目标](<../Dockerfile>)固定 x86-64-v2，不用 native，以免在 N5105 上 SIGILL。
- dummy main 依赖缓存之后必须 touch 源码并 cargo clean 本 crate；不能删强制重编步骤，否则可能运行占位二进制、零日志 exit 0。不要用二进制 grep 哨兵作为产物断言。
- [compose](<../docker-compose.yml>)使用可写目录卷 `./server-data:/data`；不要 :ro 或退回单文件配置挂载。配置、模型、插件 artifact、预设与热句落盘需保持持久化；NLU 训练另按 compose 显式挂载宿主 Docker socket，训练依赖由独立镜像提供。
- 模型下载行为与地址以[入口脚本](<../docker-entrypoint.sh>)为准：显式完整 URL 不再套代理；失败中止启动，允许缺模型只用于有意识的诊断，不代表真实引擎可用。
- 模型包必须对应[配置样例](<../server/config.example.toml>)路径，使用 model.int8.onnx；Kokoro 中文需要双语整包及词典数据，不选仅英文包。
- 保存配置失败先看启动记录的配置来源与挂载属主/权限，不把“无外部来源导致 400”误判成热刷新问题。

裸机构建与真实依赖从[服务端入口](<../server/AGENTS.md>)继续逐级阅读。
