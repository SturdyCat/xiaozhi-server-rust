# ============================================================
# xiaozhi-server-rust — Docker 多阶段构建（形态对齐 xiaoya-player 的 Dockerfile）
#
# 发布链路（两条来源，⛔ 同一 tag 双源并存会互相覆盖）：
#   ① 阿里云 ACR 源码自动构建：push 代码即自动构建（构建机在国内）。规则：Dockerfile 路径
#      `Dockerfile`、构建上下文 = 仓库根、镜像版本 `latest`、构建参数留空。
#   ② GitHub Actions 手动构建推送：.github/workflows/docker-amd64.yml（仅 linux/amd64，
#      推 registry.cn-hangzhou.aliyuncs.com/notip-com-cn/xiaozhi-server-rust，需仓库
#      secrets ACR_USERNAME/ACR_PASSWORD）。用它产出 :latest 时须关闭 ACR 自动构建规则。
# 本文件必须位于仓库根目录，切勿在 server/ 下执行
#   `docker build .`（context=server/，找不到本文件）。本机验证：仓库根 `docker build -t xiaozhi-server .`
#
# 🚨 构建内存（ACR 构建机内存有限，实测 OOM 表现：构建日志戛然而止、以
#   `rpc error: ... error reading from server: EOF` 收尾——构建容器被 OOM kill）：
#   web 阶段的三个 JVM/Node 进程（Gradle / Kotlin daemon / webpack-Node）全部显式限堆，
#   参数取自 xiaoya-player 实测口径（其 webpack 峰值 2.8-3.2GB，不限堆必被杀）。
#   上述档位已 ARG 化：**默认值 = ACR 校准档**（ACR 自动构建不传参，行为不变），
#   ⛔ 默认档不要为了「跑得更快」调高——调高只会重新触发 OOM 中断。GHA 构建
#   （ubuntu-24.04 标准机 16GB/4vCPU）由 .github/workflows/docker-amd64.yml 的
#   build-args 按大内存档覆盖，两边档位互不影响。
#
# 🚨 2026-10-27 二次复现的结论：**限堆只是必要条件，不是充分条件**。
#   ⚠️ 更正（同日第三次复现后补记）：同日另有一部分失败日志的断点与下面「第三次复现」
#   完全一致（停在 jsNpmAggregated），那部分**不是** OOM，是 COPY 漏了 sharedCompose。
#   判断口径：日志停在配置阶段末尾且无任何 Gradle 异常 → 查 COPY 集合；有 EOF/整机被杀痕迹
#   且已完成配置 → 才按内存处理。
#   崩点：web 阶段第 13 步（publishWeb）在 `:shared:compileKotlinJs` fork Kotlin daemon
#   的瞬间 EOF，之前没有任何 Gradle 异常栈（无 Java heap space / Daemon disappeared）
#   → 整机 cgroup OOM kill，而非某个堆上限被撑爆。
#   主因是**并行叠加**：BuildKit 会并行执行没有依赖关系的 stage，`cargo build --release`
#   （rustc 默认 `-j=CPU 核数`，本依赖树含 ort/tokenizers/sherpa-onnx，单进程 1.5–2.5GB）
#   与 web 的 Gradle+Kotlin daemon（约 3.5GB）同时驻留，4–8GB 档构建机直接爆。
#   因此两处硬约束（①参数化为默认 2，GHA 大内存档传 4；②任何档位都勿删）：
#     ① build 阶段 `CARGO_BUILD_JOBS`（ARG，默认 2）——不限制就按 CPU 核数开满 rustc；
#     ② build 阶段以 `COPY --from=web` 建立依赖 → 两个阶段严格串行，峰值永不同时出现
#       （GHA 16GB 下两段并跑虽勉强放得下，但叠加峰值贴近上限且省时有限，同样不放开）。
#
# 🚨 2026-10-27 第三次复现的**真正根因**（与前两条内存结论无关，别再按 OOM 调参）：
#   ACR 日志停在 `Configuration 'jsNpmAggregated' was resolved during configuration time.`
#   之后一行不出，两次完全同一个断点 → 不是随机 OOM，是确定性失败。
#   原因：web 阶段的 COPY 集合漏了 `client/sharedCompose`。settings.gradle.kts include 了
#   `:sharedCompose`，shared/build.gradle.kts 又 `implementation(project(":sharedCompose"))`，
#   容器里该模块没有构建脚本 → 空 project（`No variants exist`）→
#   `Could not determine the dependencies of task ':kotlinNpmInstall'` 必失败。
#   本地能编、ACR 必败，差异就在这里（本机 `:sharedCompose:compileKotlinJs` 有产物 195 KiB）。
#   修法：COPY 清单层与源码层都要带上 sharedCompose（见下方两处 COPY 与源码断言）。
#
# 🚨 2026-10-27 第四次复现：编译全绿后死在业务包 webpack（cgroup OOM 第二形态）。
#   断点：`:shared:jsBrowserProductionWebpack` 刚打印 webpack patch 已加载即 EOF。
#   此时配置阶段已过（sharedCompose 修复生效）、:shared 两个 Kotlin/JS 编译全部完成、
#   无任何编译错误栈 → 整机 OOM 而非堆溢出（Node 堆不足会抛明确的
#   "JavaScript heap out of memory"；EOF = 构建容器整体被杀）。
#   峰值格局：Gradle JVM(1280m) + 编译完成后仍驻留的 Kotlin daemon(1024m) + 业务包
#   webpack Node 三家叠加。material3 补丁入库（2f55b53）后 sharedCompose 源码变大、
#   业务包 webpack 输入变大，把之前勉强能过的档位顶爆。
#   修法：编译与 publishWeb 拆成两条 RUN——BuildKit 每条 RUN 是一次性容器，RUN 结束
#   所有残留进程（含 Kotlin daemon）随之销毁。第一条只做 Kotlin/JS 编译 + 壳打包
#   （Gradle+daemon 交替，本次日志已证能过）；第二条 publishWeb 时编译产物全部
#   up-to-date、不再 fork daemon，业务包 webpack 时只剩 Gradle+Node 两家。
#   ⚠️ 第五次复现真正根因（修正前几次判断）：f14abcc 误把第一条 RUN 的 `RUN …` 整段删掉，
#   只剩一条 publishWeb 全家桶（编译 + 双 webpack + copy 一次做完），Kotlin daemon + Gradle +
#   双 webpack Node 三家同驻、峰值顶到最高，业务包 webpack 直接 OOM（日志 `26 executed` 全重编）。
#   而 2632d8a 本身也漏跑业务包 webpack（只跑了壳），导致 publishWeb 重跑它才 OOM。
#   正确修法：恢复两条 RUN，且把**业务包 + 壳两个 webpack 都塞进第一条**，让第二条 publishWeb
#   只剩 copy/verify（或至多重跑 webpack，但编译 up-to-date、不 fork daemon）。见下方两条 RUN。
#   判断口径更新：本机可复现成功、而 ACR 报 BUILD FAILED 且无具体编译错误时，EOF 与
#   BUILD FAILED 都算 OOM 形态，区别仅在是否整机杀；只有出现明确编译错误（Module not found /
#   Unresolved reference / Compilation error）才属代码问题、与内存无关。
# 三个阶段：
#   web    —— Gradle 构建 Kuikly H5 管理页（client 工程：shared 业务包 + h5App 壳）
#   build  —— 编译 Rust server（--features sherpa,nlu，真实 ASR/TTS 引擎 + TinyBERT NLU）
#   运行时 —— debian-slim：同源托管 管理页(/) + /api + /api/ws，模型缺失由入口脚本自下载
#
# 运行镜像内浏览器访问 `http://<host>:8000/` 即管理页，`/api/config` 读写 SQLite 配置库（data.db），
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
# 🚨 settings.gradle.kts 里 include(":sharedCompose")，且 shared/build.gradle.kts 通过
#   implementation(project(":sharedCompose")) 依赖它。这个模块的构建脚本必须进依赖缓存层，
#   否则配置 :shared 时 `:sharedCompose` 是空 project（无脚本、无 js 产物），
#   publishWeb 解析依赖阶段必失败——见文件头 2026-10-27 第三次复现记录。
COPY client/sharedCompose/build.gradle.kts client/sharedCompose/build.gradle.kts
COPY client/apps/h5App/build.gradle.kts client/apps/h5App/build.gradle.kts
WORKDIR /src/client
# 只做配置/下载依赖，不编译业务代码（保持轻）
RUN ./gradlew :apps:h5App:help --no-daemon

# 业务源码：shared（KMP 共享层，@Page 页面 + Kuikly 插件打业务包）+ sharedCompose + h5App 壳
# ⚠️ 上方 WORKDIR 已切到 /src/client（上面那个 help 任务的需要），COPY 的 dest 相对 WORKDIR 解析：
#    必须写 ./shared、./sharedCompose、./apps/h5App（= Gradle 根 /src/client 下的模块目录，
#    对应 settings 的 :shared、:sharedCompose、:apps:h5App）。曾误写 ./client/shared，源码实际
#    落盘 /src/client/client/shared（双层嵌套）：COPY 静默成功，Gradle 侧 shared/ 只有
#    build.gradle.kts 没有 src → js 源集 NO-SOURCE → KSP 不生成 KuiklyCoreEntry.kt →
#    打包任务深处 FileNotFoundException。
COPY client/shared ./shared
# sharedCompose：:shared 的依赖模块（Kuikly Compose 组件层 + 上游 compose-settings 补丁）。
# ⚠️ 漏掉这一步就是「本地能编、ACR 必败」：settings 里 include 了它，缺源码则 :shared 的
#    js 依赖链断在这里，日志停在配置阶段末尾（典型末行 `Configuration 'jsNpmAggregated'
#    was resolved during configuration time.`），既无 OOM 也无编译错误。
COPY client/sharedCompose ./sharedCompose
COPY client/apps/h5App ./apps/h5App

# 🚨 源码断言：验证业务源码真的落在了 Gradle 工程期望的位置（而非嵌套错位或上下文缺目录）。
#   缺失/错位时 Gradle 会把 js 源集判成 NO-SOURCE → KSP 不执行 → KuiklyCoreEntry.kt 不生成
#   → 打包任务深处抛晦涩的 FileNotFoundException（实测部署链路 409s 才失败）。这里在进入
#   Gradle 前快速失败，错误信息直指根因。
RUN if ! (test -d shared/src/commonMain/kotlin \
          && test -f shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigPage.kt \
          && test -f sharedCompose/src/commonMain/kotlin/com/alorma/compose/settings/ui/SettingsGroup.kt \
          && test -f apps/h5App/src/jsMain/kotlin/com/xiaozhi/admin/Main.kt); then \
      echo "❌ web 构建缺业务源码：/src/client/{shared,sharedCompose,apps/h5App}/src 未就位（COPY 目标路径错位或上下文不完整）" >&2; \
      exit 1; \
    fi

# 🚨 内存/并行档位（ARG 化，默认 = ACR 4-8GB 构建机校准档，见文件头，勿调高默认值）：
#   GRADLE_XMX           Gradle JVM 堆（两条 RUN 共用；orchestrator 用不了多大堆）
#   KOTLIN_DAEMON_XMX    Kotlin/JS 编译 daemon 堆（随 RUN 容器结束销毁）
#   NODE_MAX_OLD_SPACE   webpack Node 的 V8 老生代上限（MB）
#   GRADLE_MAX_WORKERS   任务级并发：org.gradle.parallel=false 只管跨 project 并行，
#                        项目内 worker API（KSP、双 webpack、verifyAdminIcons）仍按此开线程，
#                        多线程同时持有 AST/IR / bundle 会把峰值再叠一截。
#   GHA（16GB/4vCPU）经 .github/workflows/docker-amd64.yml build-args 覆盖为放宽档。
#   -PwebSourceMap=false：生产镜像不调试，关掉 .map 生成（xiaoya 实测省 ~400MB 峰值内存）。
ARG GRADLE_XMX=1024m
ARG KOTLIN_DAEMON_XMX=1024m
ARG NODE_MAX_OLD_SPACE=1024
ARG GRADLE_MAX_WORKERS=1
#
# 🚨 两条 RUN 拆分（见文件头第四次/第五次复现记录，勿合并回一条）：BuildKit 每条 RUN 是一次性
#   容器，RUN 结束所有残留进程（含 Kotlin daemon）随之销毁。第一条把「编译 + 两个 webpack」全做掉——
#   业务包 :shared:jsBrowserProductionWebpack 与壳 :apps:h5App:jsBrowserProductionWebpack 都必须在
#   本条跑，不能留给 publishWeb 重跑（2632d8a 漏跑业务包 webpack，第二条重跑它时 OOM）。
#   两处 **影响产物的 -P 参数（仅 -PwebSourceMap）必须与下一条 publishWeb 完全一致**，否则
#   up-to-date 失效、第二条会重新 fork Kotlin daemon，拆分白做。JVM 堆参数（-Xmx /
#   -Dkotlin.daemon.jvm.options）属 Gradle 自身开销、不进任务指纹，可按阶段独立调，无需一致。
#   --max-workers（ACR 档=1）：强制编译与两个 webpack 串行，避免双 webpack 并行把峰值再叠
#   一截（cgroup 小规格顶不住）；大内存档可放宽到 2。
RUN ./gradlew :shared:compileKotlinJs :shared:compileProductionExecutableKotlinJs \
      :shared:jsBrowserProductionWebpack :apps:h5App:jsBrowserProductionWebpack \
      --no-daemon --max-workers=${GRADLE_MAX_WORKERS} -PwebSourceMap=false \
      -Dorg.gradle.jvmargs="-Xmx${GRADLE_XMX} -Dfile.encoding=UTF-8" \
      -Dkotlin.daemon.jvm.options=-Xmx${KOTLIN_DAEMON_XMX}

# 🚨 第二条 RUN（publishWeb 收尾，第五次复现 OOM 点按单进程处理）：编译 + 两个 webpack 已
#   up-to-date（第一条跑过同名任务），本条只重新执行聚合任务本身 + 三份 sync Copy + verifyAdminIcons，
#   不重跑 webpack、不 fork Kotlin daemon（本条 Gradle 仅是 orchestrator）。即使个别 webpack
#   任务被判定需重跑，因编译 up-to-date 不会重新 fork daemon，峰值也只是 Gradle+Node 两家，
#   远低于全家桶同驻。--max-old-space-size 是 V8 堆上限，约束 JS 堆而非 Node 全部 RSS：
#   ACR 档 1024 下调高只会让 Node RSS 更大更易爆 cgroup，调低则 Node 先 heap OOM 报错；
#   大内存档经 build-args 放宽（xiaoya 实测 webpack 峰值 2.8-3.2GB，GHA 档 3072）。
RUN NODE_OPTIONS="--max-old-space-size=${NODE_MAX_OLD_SPACE}" \
    ./gradlew :apps:h5App:publishWeb --no-daemon --max-workers=${GRADLE_MAX_WORKERS} -PwebSourceMap=false \
      -Dorg.gradle.jvmargs="-Xmx${GRADLE_XMX} -Dfile.encoding=UTF-8" \
 && test -f apps/h5App/web/index.html \
 && test -f apps/h5App/web/nativevue2.js \
 && test -f apps/h5App/web/h5App.js

# ---- Rust 构建阶段 ----
# rust:1.92-bookworm = 满足 ort 2.0.0-rc.14（nlu 特性）的 rustc ≥ 1.92 最低要求
# （rust:1.90 会报「ort requires rustc 1.92」直接构建失败）。
# 保持 Debian（非 alpine）：sherpa-onnx 预编译原生库与 audiopus/libopus 按 glibc 链接，
# 与 xiaoya 的 musl 静态方案不通用，勿照搬。
FROM rust:1.92-bookworm AS build

# pkg-config + libopus-dev：audiopus_sys 需要（libopus 编译/链接）
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libopus-dev \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 🚨 阶段串行屏障（见文件头 2026-10-27 记录，勿删）：BuildKit 默认并行执行没有依赖关系的
#   stage，cargo 的 rustc 进程群会与 web 阶段的 Gradle / Kotlin daemon / webpack 同时驻留，
#   4–8GB 档构建机必被 OOM kill（日志以 `rpc error ... EOF` 收尾，无任何编译错误）。
#   这里读一个 web 阶段产物建立 DAG 依赖：web 先跑完，cargo 才开始；文件落 /tmp，不进运行镜像。
#   代价：总时长 ≈ web + build（原先并行取 max），换峰值内存永不同时出现。
COPY --from=web /src/client/apps/h5App/web/index.html /tmp/.web-stage-done

# 🚨 rustc 并行度上限（ACR 档默认 2，勿删）：cargo 默认 `-j=CPU 核数`，本依赖树含 ort-sys /
#   tokenizers / sherpa-onnx-sys（自带 C++/protobuf codegen），单 rustc 峰值 1.5–2.5GB，
#   -j4 就能吃掉 8GB 机器的全部内存。2 = 实测能过 ACR 构建机的档位（慢一点，但不会中断）；
#   GHA（16GB/4vCPU）经 build-args 传 4 对齐核数——上方串行屏障保证与 web 峰值错开，
#   4 路 rustc 并发（最坏 ~10GB）16GB 放得下。ARG 对本阶段后续 RUN 即环境变量，cargo 直读。
ARG CARGO_BUILD_JOBS=2

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
 && cargo build --release --features sherpa,nlu --locked \
 && rm -rf src

# 真实源码（bin-only crate：入口即 src/main.rs，无 lib.rs）
COPY server/src ./src
# 编译期内嵌资源：bench_api 样本经 include_bytes!（src/app/bench_api/../../../test_assets）
# 在编译期读 /app/test_assets/，缺了 cargo 直接编译失败（ACR 2026-10-21 实测）。
COPY server/test_assets ./test_assets

# ⚠️ 强制重编译本 crate ——「容器零日志 exit 0」事故的根因所在，勿简化这条 RUN。
#   ① 根因（mtime 陷阱）：COPY 保留构建上下文里文件的 mtime（ACR clone 代码的时刻），它早于
#      上一步 dummy 在容器内的编译时刻 → cargo 指纹判定「源码比产物旧 = 没改过」，真源码被
#      静默跳过：输出 `Finished ... in 0.00s`，target 里留的是占位 `fn main() {}` → 镜像构建
#      成功，容器却启动即退出（xiaoya 实测复现，cargo 1.90）。
#   ② 修法：touch 把源码 mtime 刷到当前 + `cargo clean -p` 直接删掉本 crate 指纹与产物
#      （不依赖 mtime 判定，依赖全保留 → 日常仍只重编本 crate）。
#   ③ 不要再加「产物断言」（曾用 grep 在二进制里找哨兵串验证非 dummy）：断言依赖构建机
#      grep 实现对二进制的匹配行为（GNU/ugrep 别名差异），ACR 上两次假阴性、越修越错，
#      已随版本戳特性一并移除（2026-10）；「Compiling + 非 0.00s 耗时」日志即重编证据。
RUN find src -type f -exec touch {} + \
 && cargo clean -p xiaozhi-server-rust --release \
 && cargo build --release --features sherpa,nlu --locked \
 && strip target/release/xiaozhi-server-rust

# ---- 运行阶段 ----
FROM debian:bookworm-slim AS runtime

# 日志时间戳本地化：tzdata 提供时区库，TZ 缺省 Asia/Shanghai
#   （chrono Local 按 TZ → /usr/share/zoneinfo 解析，app::logtime 统一出口；
#   部署在其他时区时用 compose 的 environment: TZ=… 覆盖即可，不必改镜像）。
ENV TZ=Asia/Shanghai

# libopus0：运行期动态链接（audiopus）；curl/bzip2：entrypoint 缺模型时按需下载解包。
# docker CLI：NLU 模型训练走宿主 docker（/var/run/docker.sock 由 compose 挂入），
#   只 run 专用训练镜像（依赖在镜像构建期装好，见 tools/nlu-trainer/Dockerfile）；
#   只装 CLI 不装 daemon（静态二进制，避免 docker.io 全家桶拖大 slim 镜像）。
# ONNX Runtime 1.22.1 动态库：nlu 特性下 ort 以 load-dynamic 运行时 dlopen——
#   构建期 ort-sys 不再静态链 onnxruntime/onnx（否则与 sherpa-onnx-sys 的静态 onnx
#   对象符号重复，ld 报 duplicate symbol）。1.22.1 的 C API(22) 满足 ort rc.14 的最低
#   API 17，向下兼容。GitHub Release 已不附 Linux 包，改从 NuGet 取（包内
#   runtimes/linux-x64/native/，未带版本号文件名恰可直接作 ORT_DYLIB_PATH）；
#   nupkg 含全平台（~128MB）但只解出 Linux 两件，镜像实际只增 ~21MB。
RUN apt-get update \
 && apt-get install -y --no-install-recommends libopus0 ca-certificates curl bzip2 unzip tzdata \
 && rm -rf /var/lib/apt/lists/* \
 && curl -fsSL https://download.docker.com/linux/static/stable/x86_64/docker-27.3.1.tgz \
      | tar xz --strip-components=1 -C /usr/local/bin docker/docker \
 && curl -fsSL -o /tmp/ort.nupkg \
      https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/1.22.1/microsoft.ml.onnxruntime.1.22.1.nupkg \
 && unzip -j /tmp/ort.nupkg 'runtimes/linux-x64/native/*' -d /usr/local/lib \
 && rm /tmp/ort.nupkg \
 && ldconfig /usr/local/lib

WORKDIR /app
# Rust server 二进制
COPY --from=build /app/target/release/xiaozhi-server-rust /app/server
# 同源托管的管理页静态产物（server [server].admin_dir 由下方 ENV 指到 /app/web）
COPY --from=web /src/client/apps/h5App/web /app/web
COPY docker-entrypoint.sh /app/docker-entrypoint.sh
# 内置配置示例：字段说明参照（配置真源为 SQLite data.db；示例不再作为首启配置生成）
COPY server/config.example.toml /app/config.example.toml
RUN chmod +x /app/docker-entrypoint.sh

# 数据卷：模型持久化挂载点（entrypoint 缺失时自动下载到此；必须可写，勿 :ro 挂载）
RUN mkdir -p /data
VOLUME ["/data"]

# 运行镜像内置默认值——只放「容器路径」类必须项，避免与代码默认值漂移：
#   XIAOZHI_CONFIG_DB：配置库文件（/data 内；空库首启以内置默认启动并落库）
#   XIAOZHI_ADMIN_DIR：管理页静态目录覆盖（见 main.rs load_config），指向上方 /app/web
#   XIAOZHI_AUTO_DOWNLOAD_MODELS：missing(默认,缺失才下) | force | off
#   （模型统一从 HuggingFace 直连下载，无需代理；SENSEVOICE_URL/KOKORO_URL/SILERO_VAD_URL 可覆盖为内网镜像地址）
#   OWW_VAD_URL 只被 server 进程读取：下发到 EchoMuse 设备的语音门模型（Silero v5，
#   官方来源 openWakeWord v0.5.1 release），且仅在 [echomuse].enabled=true 时下载。
# 注意：不得内置 RUST_LOG 默认值——RUST_LOG 显式存在时 main.rs 的日志过滤兜底
#   （info + ort::logging 压 warn，见 default_log_filter）永不生效，ort 明细会刷屏。
#   需要调级别时在 compose 的 environment 里按需设置 RUST_LOG。
ENV XIAOZHI_CONFIG_DB=/data/data.db \
    XIAOZHI_ADMIN_DIR=/app/web \
    XIAOZHI_AUTO_DOWNLOAD_MODELS=missing \
    ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so
# 8000 = 主端口（[server].port）；8767 = EchoMuse 控制器专用端口（[echomuse].plain_port）。
# host 网络部署（docker-compose 默认）下 EXPOSE 仅作文档，监听端口即宿主端口。
EXPOSE 8000 8767

# 健康检查：curl 已随运行层安装；/api/health 不依赖模型就绪，start-period 20s 足够。
# 端口读服务启动时落盘的 /data/.runtime_port（见 main.rs），改 [server].port 无需同步这里；
# 文件缺失（如旧版镜像的挂载卷）时兜底默认 8000。
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -f http://127.0.0.1:$(cat /data/.runtime_port 2>/dev/null || echo 8000)/api/health || exit 1

ENTRYPOINT ["/app/docker-entrypoint.sh"]
