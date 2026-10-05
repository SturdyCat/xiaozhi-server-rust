# ============================================================
# xiaozhi-server-rust — Docker 多阶段构建（形态对齐 xiaoya-player 的 Dockerfile）
#
# 发布链路：镜像由**阿里云 ACR 源码自动构建**产出（push 代码即自动构建，构建机在国内）。
#   ACR 构建规则：Dockerfile 路径 `Dockerfile`、构建上下文（Dockerfile 目录）= 仓库根、
#   镜像版本 `latest`、构建参数留空。本文件必须位于仓库根目录，切勿在 server/ 下执行
#   `docker build .`（context=server/，找不到本文件）。本机验证：仓库根 `docker build -t xiaozhi-server .`
#
# 🚨 构建内存（ACR 构建机内存有限，实测 OOM 表现：构建日志戛然而止、以
#   `rpc error: ... error reading from server: EOF` 收尾——构建容器被 OOM kill）：
#   web 阶段的三个 JVM/Node 进程（Gradle / Kotlin daemon / webpack-Node）全部显式限堆，
#   参数取自 xiaoya-player 实测口径（其 webpack 峰值 2.8-3.2GB，不限堆必被杀）。
#   ⛔ 不要为了「跑得更快」调高上限——调高只会重新触发 OOM 中断。
#
# 三个阶段：
#   web    —— Gradle 构建 Kuikly H5 管理页（client 工程：shared 业务包 + h5App 壳）
#   build  —— 编译 Rust server（--features sherpa，真实 ASR/TTS 引擎）
#   运行时 —— debian-slim：同源托管 管理页(/) + /api + /api/ws，模型缺失由入口脚本自下载
#
# 运行镜像内浏览器访问 `http://<host>:8000/` 即管理页，`/api/config` 读写 config.toml，
# `/api/ws` 供设备（xiaozhi-esp32）WebSocket 接入；静态页与 API 同源，无 CORS 问题。
# ============================================================

# ---- web 阶段：构建 Kuikly H5 管理页静态产物（client Gradle 工程）----
# `gradle:8.14.5-jdk17-jammy`：与 client/gradle/wrapper 完全同版本（8.14.5）+ JDK17。
# 用 ./gradlew（wrapper）跑，与本地/ACR 实测路径一致；wrapper 从腾讯镜像下载发行包（国内可直连）。
# ⚠️ 无需 Android SDK：shared 声明了 com.android.library，但 publishWeb 链路只走 js 目标，
#    AGP 仅在配置阶段加载、不真编译 Android（xiaoya-player 同形态，空 ANDROID_HOME 实测通过）。
FROM gradle:8.14.5-jdk17-jammy AS web

WORKDIR /src

# 依赖缓存层：先 COPY 构建清单（settings + 根脚本 + gradle.properties + wrapper + 各模块脚本
# + Kotlin/JS 的 npm 依赖锁），用一个最轻任务把插件/依赖全部下载进镜像层；
# 之后 COPY 业务源码不再重复拉依赖（冷构建 ~1.6GB 依赖只发生一次）。
# ⚠️ kotlin-js-store（yarn.lock）必须带：没有 lockfile 时 yarn 自行解析传递依赖，
#    与仓库锁定值漂移（xiaoya 实测 @types/node 26.4.1 → 26.6.2）。
COPY client/gradlew client/gradlew.bat client/gradle.properties client/build.gradle.kts client/settings.gradle.kts ./client/
COPY client/gradle ./client/gradle
COPY client/kotlin-js-store ./client/kotlin-js-store
COPY client/shared/build.gradle.kts client/shared/build.gradle.kts
COPY client/apps/h5App/build.gradle.kts client/apps/h5App/build.gradle.kts
WORKDIR /src/client
# 只做配置/下载依赖，不编译业务代码（保持轻）
RUN ./gradlew :apps:h5App:help --no-daemon

# 业务源码：shared（KMP 共享层，@Page 页面 + Kuikly 插件打业务包）+ h5App 壳
# ⚠️ 上方 WORKDIR 已切到 /src/client（第 45 行 RUN 的需要），COPY 的 dest 相对 WORKDIR 解析：
#    必须写 ./shared、./apps/h5App（= Gradle 根 /src/client 下的模块目录，对应 settings 的
#    :shared、:apps:h5App）。曾误写 ./client/shared，源码实际落盘 /src/client/client/shared
#    （双层嵌套）：COPY 静默成功，Gradle 侧 shared/ 只有 build.gradle.kts 没有 src →
#    js 源集 NO-SOURCE → KSP 不生成 KuiklyCoreEntry.kt → 打包任务深处 FileNotFoundException。
COPY client/shared ./shared
COPY client/apps/h5App ./apps/h5App

# 🚨 源码断言：验证业务源码真的落在了 Gradle 工程期望的位置（而非嵌套错位或上下文缺目录）。
#   缺失/错位时 Gradle 会把 js 源集判成 NO-SOURCE → KSP 不执行 → KuiklyCoreEntry.kt 不生成
#   → 打包任务深处抛晦涩的 FileNotFoundException（实测部署链路 409s 才失败）。这里在进入
#   Gradle 前快速失败，错误信息直指根因。
RUN if ! (test -d shared/src/commonMain/kotlin \
          && test -f shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigPage.kt \
          && test -f apps/h5App/src/jsMain/kotlin/com/xiaozhi/admin/Main.kt); then \
      echo "❌ web 构建缺业务源码：/src/client/shared/src 或 /src/client/apps/h5App/src 未就位（COPY 目标路径错位或上下文不完整）" >&2; \
      exit 1; \
    fi

# 🚨 内存三上限（见文件头说明，勿调高）：Gradle JVM 1280m / Kotlin daemon 1024m / webpack Node 1024m；
#   -PwebSourceMap=false：生产镜像不调试，关掉 .map 生成（xiaoya 实测省 ~400MB 峰值内存）。
# 产物断言：web/ 下三件套缺一不可（index.html 入口 / nativevue2.js 业务包 / h5App.js 壳），
#   缺失即构建失败——假产物不得静默出厂（部署后表现为白屏/404，排查成本远高于此处一行 test）。
RUN NODE_OPTIONS="--max-old-space-size=1024" \
    ./gradlew :apps:h5App:publishWeb --no-daemon -PwebSourceMap=false \
      -Dorg.gradle.jvmargs="-Xmx1280m -Dfile.encoding=UTF-8" \
      -Dkotlin.daemon.jvm.options=-Xmx1024m \
 && test -f apps/h5App/web/index.html \
 && test -f apps/h5App/web/nativevue2.js \
 && test -f apps/h5App/web/h5App.js

# ---- Rust 构建阶段 ----
# rust:1.90-bookworm = 与生成 Cargo.lock 的本机 cargo 1.90 对齐（勿降级，见 agents.md §5.5）。
# 保持 Debian（非 alpine）：sherpa-onnx 预编译原生库与 audiopus/libopus 按 glibc 链接，
# 与 xiaoya 的 musl 静态方案不通用，勿照搬。
FROM rust:1.90-bookworm AS build

# pkg-config + libopus-dev：audiopus_sys 需要（libopus 编译/链接）
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libopus-dev \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 目标指令集固定为 x86-64-v2：部署机 N5105（Tremont，无 AVX）可安全运行。
# 不要用 native —— ACR 构建机的 CPU 与部署机不同，native 会嵌入部署机不支持的
# AVX/AVX2 指令，导致运行时 SIGILL 崩溃。
ENV RUSTFLAGS="-C target-cpu=x86-64-v2"

# 依赖缓存层：先 COPY 清单 + dummy main.rs 编译全部依赖（含 sherpa 特性的原生库下载——
# 网络耗时都落在这一层，日常改代码只重编本 crate）。--locked：清单与 lock 不一致即构建失败
# （可复现发布镜像是核心诉求）。
COPY server/Cargo.toml server/Cargo.lock ./
RUN mkdir -p src \
 && echo 'fn main() {}' > src/main.rs \
 && cargo build --release --features sherpa --locked \
 && rm -rf src

# 真实源码（bin-only crate：入口即 src/main.rs，无 lib.rs）
COPY server/src ./src

# ⚠️ 强制重编译本 crate + 产物断言 ——「容器零日志 exit 0」事故的根因所在，勿简化这条 RUN。
#   ① 根因（mtime 陷阱）：COPY 保留构建上下文里文件的 mtime（ACR clone 代码的时刻），它早于
#      上一步 dummy 在容器内的编译时刻 → cargo 指纹判定「源码比产物旧 = 没改过」，真源码被
#      静默跳过：输出 `Finished ... in 0.00s`，target 里留的是占位 `fn main() {}` → 镜像构建
#      成功，容器却启动即退出（xiaoya 实测复现，cargo 1.90）。
#   ② 修法：touch 把源码 mtime 刷到当前 + `cargo clean -p` 直接删掉本 crate 指纹与产物
#      （不依赖 mtime 判定，依赖全保留 → 日常仍只重编本 crate）。
#   ③ 断言：哨兵串取 ws.rs 健康检查的字面量（dummy 占位二进制不可能含）；注意 dummy 也链接
#      sherpa 原生库、体积与真二进制同量级，故**不能用大小阈值区分**，只能靠哨兵串。
RUN find src -type f -exec touch {} + \
 && cargo clean -p xiaozhi-server-rust --release \
 && cargo build --release --features sherpa --locked \
 && grep -q "xiaozhi-server-rust ok" target/release/xiaozhi-server-rust \
 && strip target/release/xiaozhi-server-rust

# ---- 运行阶段 ----
FROM debian:bookworm-slim AS runtime

# libopus0：运行期动态链接（audiopus）；curl/bzip2：entrypoint 缺模型时按需下载解包
RUN apt-get update \
 && apt-get install -y --no-install-recommends libopus0 ca-certificates curl bzip2 \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
# Rust server 二进制
COPY --from=build /app/target/release/xiaozhi-server-rust /app/server
# 同源托管的管理页静态产物（server [server].admin_dir 由下方 ENV 指到 /app/web）
COPY --from=web /src/client/apps/h5App/web /app/web
COPY docker-entrypoint.sh /app/docker-entrypoint.sh
RUN chmod +x /app/docker-entrypoint.sh

# 数据卷：模型持久化挂载点（entrypoint 缺失时自动下载到此；必须可写，勿 :ro 挂载）
RUN mkdir -p /models
VOLUME ["/models"]

# 运行镜像内置默认值——只放「容器路径」与「生产日志策略」两类必须项，避免与代码默认值漂移：
#   XIAOZHI_CONFIG：容器内配置文件（compose 挂载 server/config.example.toml 即可开箱即用）
#   XIAOZHI_ADMIN_DIR：管理页静态目录覆盖（见 main.rs load_config），指向上方 /app/web
#   XIAOZHI_AUTO_DOWNLOAD_MODELS：missing(默认,缺失才下) | force | off
#   （模型统一从 HuggingFace 直连下载，无需代理；SENSEVOICE_URL/KOKORO_URL/SILERO_VAD_URL 可覆盖为内网镜像仓库 ID）
ENV XIAOZHI_CONFIG=/etc/xiaozhi/config.toml \
    XIAOZHI_ADMIN_DIR=/app/web \
    XIAOZHI_AUTO_DOWNLOAD_MODELS=missing \
    RUST_LOG=info

EXPOSE 8000

# 健康检查：curl 已随运行层安装；/api/health 不依赖模型就绪，start-period 20s 足够。
# 端口 8000 与 config.example.toml 的 listen 默认值一致——若自定义 [server].listen 需同步改这里。
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -f http://127.0.0.1:8000/api/health || exit 1

ENTRYPOINT ["/app/docker-entrypoint.sh"]
