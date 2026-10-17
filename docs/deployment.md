# Docker 部署与模型准备

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

## 9. Docker 部署细节

### 模型自动下载

容器入口（`docker-entrypoint.sh`）启动时检测关键模型文件（`silero_vad.onnx`、`SenseVoiceSmall/tokens.txt`、`Kokoro/model.int8.onnx`）：

- **缺失则自动下载**到挂载的 `/data/models`（从官方 GitHub Release 拉整包 tar.bz2 本地解压，幂等 + 断点续传）。
- 默认直连原始地址；compose 配置 `GITHUB_PROXY` 才走代理。下载地址可用 `SENSEVOICE_URL` / `KOKORO_URL` / `SILERO_VAD_URL` 覆盖为完整直链（内网镜像）。
- 行为开关 `XIAOZHI_AUTO_DOWNLOAD_MODELS`：`missing`（默认）/ `force` / `off`。

> 不手动预置模型也能直接 `docker compose up`。离线/内网环境：先 `SENSEVOICE_URL=<内网镜像> ./server/scripts/download_models.sh /host/data/models` 预置再挂载，或设 `XIAOZHI_AUTO_DOWNLOAD_MODELS=off`。

### 部署机指令集

构建期固定 `RUSTFLAGS="-C target-cpu=x86-64-v2"`（见 `Dockerfile`），兼容 N5105（Tremont，无 AVX）。**不要用 `native`**——会在构建机嵌入部署机不支持的 AVX 指令，运行时 `SIGILL` 崩溃。

### 真实引擎（`sherpa` feature）本地依赖

1. 系统 `libopus`；2. 网络（构建期原生库下载）；3. 本地模型文件：

```bash
./server/scripts/download_models.sh /host/data/models     # 裸机部署时手动下载
cd server && ~/.cargo/bin/cargo run --release --features sherpa -- --config-db /host/data/data.db
```

## 构建与故障边界

- [Dockerfile](<../Dockerfile>)必须以仓库根为上下文；Web 阶段严格保留源码/产物断言、Gradle/Kotlin/Node 三项内存上限和关闭 source map 的生产配置，不为加速提高堆上限。
- Rust 镜像版本与[锁文件](<../server/Cargo.lock>)同步，不随意降级；[构建目标](<../Dockerfile>)固定 x86-64-v2，不用 native，以免在 N5105 上 SIGILL。
- dummy main 依赖缓存之后必须 touch 源码并 cargo clean 本 crate；不能删强制重编步骤，否则可能运行占位二进制、零日志 exit 0。不要用二进制 grep 哨兵作为产物断言。
- [compose](<../docker-compose.yml>)使用可写目录卷 `./server-data:/data`；不要 :ro 或退回单文件配置挂载。配置、模型、固件、预设与热句落盘需保持持久化。
- 模型下载行为与地址以[入口脚本](<../docker-entrypoint.sh>)为准：显式完整 URL 不再套代理；失败中止启动，允许缺模型只用于有意识的诊断，不代表真实引擎可用。
- 模型包必须对应[配置样例](<../server/config.example.toml>)路径，使用 model.int8.onnx；Kokoro 中文需要双语整包及词典数据，不选仅英文包。
- 保存配置失败先看启动记录的配置来源与挂载属主/权限，不把“无外部来源导致 400”误判成热刷新问题。

裸机构建与真实依赖从[服务端入口](<../server/AGENTS.md>)继续逐级阅读。
