# syntax=docker/dockerfile:1

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
#     ② build 阶段以 `COPY --from=<静态页 stage>` 建立依赖（W15-1 起为 admin-web，原先为 web）
#       → 两个阶段严格串行，峰值永不同时出现
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
# 🚨 2026-10-31 第五次复现：报错形态回到第三次（COPY `"/client/sharedCompose": not found`），
#   但根因**反转**：`638849b`（GearUI 全量迁移）已删除整个 :sharedCompose 模块——
#   Kuikly 升 2.30.0 上游补齐了 material3 缺失 API，补丁按预案删除（settings/build 脚本
#   同步清理，本地三端编译全绿可证），但本文件的 COPY 与源码断言**没有同步删除**，
#   COPY 一个仓库里已不存在的目录 → buildx 直接失败。修法：删除全部 sharedCompose
#   COPY/断言/注释（本轮），后续若再引入新模块，COPY 清单与构建脚本必须同一提交内同步。
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
#
# 🚨 2026-11-07 第六次复现：**明确编译错误，既不是 OOM 也不是「漏 COPY」**，别再按内存调参：
#   `error: couldn't read 'src/../../plugins/mod.rs': No such file or directory`
#   --> src/main.rs:26:1（build 12/12，cargo 刚打印 Compiling 即失败，exit 101）。
#   根因：`3802aa3` 把插件源码从 `server/plugins/` 提到仓库根 `plugins/`，main.rs 的
#   `#[path]` 同步由 `../plugins/mod.rs` 改成 `../../plugins/mod.rs`（本地 server/src/../..
#   = 仓库根，编译通过）；但本文件只把 COPY 的**源**从 `server/plugins` 换成 `plugins`，
#   目标仍写 `./plugins`。容器里 Cargo manifest 在 /app（对应本地 server/）、main.rs 在
#   /app/src，于是 `src/../../plugins` 解析到容器根 `/plugins`，而文件实际落在 `/app/plugins`，
#   少一层 → rustc 打不开。实证：最小工程把 plugins 放 crate 根上一层可编译，放 src 同级即报同错；
#   `find src plugins` 能跑通只证明 /app/plugins 存在，不证明 #[path] 指向的位置正确。
#   修法：`COPY plugins /plugins`（插件目录落在 crate 根的上一层，与本地「plugins/ 与 server/
#   平级」语义一致），find 清单同步改成 `/plugins`；源码层的 #[path] 保持不动（改它本地就编译不过）。
#   ⛔ 与第五次同型：移动源码目录时，`#[path]` 层数 / COPY 目标 / find 清单必须同一提交内同步。
#   判断口径补充：日志出现 `couldn't read <相对路径>` 这类明确编译错误时，先按 rustc 的
#   `#[path]` / `include_*!` 相对层级手算容器内绝对路径，再决定改 COPY 目标还是改源码。
#
# 🚨 2026-11-08 第七次复现：**GHA 死在 web 15/16，既不是 OOM / 漏 COPY，也不是编译错误**：
#   `Could not determine the dependencies of task ':kotlinYarnSetup'` →
#   `Could not HEAD https://github.com/yarnpkg/yarn/releases/download/v1.22.17/yarn-v1.22.17.tar.gz`
#   `Received status code 504`（同日 sherpa-onnx-sys 的 GitHub release 端点也 504，见 build 阶段预取层）。
#   根因：KGP 2.1.21 的 AbstractSetupTask.withUrlRepo 在 :kotlinYarnSetup 求 @Classpath 输入时，
#   把 GitHub ivy 仓库**临时**注册进根工程 project.repositories（解析完即删）；PREFER_PROJECT 下
#   project 一旦声明仓库，settings.gradle.kts 的整条仓库链就不参与该 detached configuration，
#   于是 838db3e 加在 settings 里的 npm 备援源从未被尝试（本地诊断实测：仓库链只剩 GitHub 一个）。
#   修法：client/build.gradle.kts 用 plugins.withType<YarnPlugin> 清空 downloadBaseUrl →
#   withUrlRepo 不注册仓库，解析回落到 settings 中两个带 content 过滤的 npm ivy 源（提到链首）；
#   npm registry 的 yarn-1.22.17.tgz 与 GitHub 归档逐字节同源（sha1 bf910747…），本地冷缓存
#   实测 5s 下载成功，Node 的 nodejs.org 仓库由 NodeJsSetupTask 独立注入、不受影响。
#   ⚠️ 判断口径补充：`Could not determine the dependencies of task` + 外部资产端点 5xx 属
#     「构建依赖下载链」问题：先查该任务的仓库注册层级（project 级动态仓库会挤掉 settings 链），
#     ⛔ 不要按 OOM 调参，也不要只在 settings 里追加仓库（不会被尝试）。
# ♻️ 构建缓存（2026-10-28 起，两层正交，详见 workflow 与本文件各 RUN 的 --mount）：
#   L1 层缓存 —— GHA workflow 以 buildx type=gha,mode=max 跨次构建复用全部 stage 的层：
#     只改 server/ → web 阶段整段命中不重编；只改 client/ → Rust 阶段命中；
#     清单不动 → 依赖下载层命中。
#   L2 内容缓存 —— RUN --mount=type=cache：Gradle 用户主目录（web 三条 RUN 共挂一块）与
#     cargo registry/git（Rust 两条 cargo RUN 共挂）放命名缓存，层失效（如改了清单）时
#     依赖/插件不再重复下载；Gradle 发行版由基础镜像提供。⚠️ 产物 target 仍烙进镜像层（dummy 依赖层
#     设计不变），挂缓存的只有下载与配置类内容——依赖编译产物进层，源码层才能增量。
#   首次构建冷建无缓存，第二次起生效。GHA 仓库级缓存配额 10GB（LRU 逐出；超限只是退化
#   成冷建，不会失败）。cache-mount 需要 BuildKit（顶部 # syntax 已声明）；ACR 自动构建
#   若重新启用，须先确认其构建机支持 BuildKit，否则在 RUN --mount 处解析报错。
# 阶段（W15-1 软下线第一波：生产 DAG = admin-web → build → runtime）：
#   web    —— [保留定义，不再进入生产 DAG] Gradle 构建 Kuikly H5 管理页（旧端回滚链）
#   admin-web —— node/pnpm 构建 React SPA（client/admin-web/dist，含 /em/ 白名单 4 文件）
#   build  —— 编译 Rust server（--no-default-features --features sherpa,nlu,wasm-plugins，模型/音频运行时 + NLU 训练/推理 + WASM 宿主）
#   运行时 —— debian-slim：同源托管 管理页(/) + /api + /api/ws，模型缺失由入口脚本按需下载
#   运行镜像包含底座、WASM 宿主与官方系统级 WASM 插件；用户插件仍经数据卷/管理端上传。
#   NLU 训练经受限的宿主 Docker 编排。
#
# 运行镜像内浏览器访问 `http://<host>:8000/` 即管理页，`/api/config` 读写 SQLite 配置库（data.db），
# `/api/ws` 供设备（xiaozhi-esp32）WebSocket 接入；静态页与 API 同源，无 CORS 问题。
#
# 🆕 W15-1（软下线第一波）：生产管理页静态产物改由 admin-web stage 自建的 dist/ 提供。
#   web（Gradle）阶段的**定义与全部旧端源码原位保留**，但文件里不再有任何
#   `COPY --from=web` 引用，BuildKit 会跳过该阶段 ⇒ 生产镜像不再依赖旧端产物/
#   Gradle 构建；回滚时仍可在本地重建旧端链（见 w15-production-build-research.md）。
#   配套：.dockerignore 放开 admin-web/scripts + public/logo.png 的排除例外；
#   .github/workflows/docker-amd64.yml 的 GRADLE_* / NODE_MAX_OLD_SPACE build-args
#   按派发要求不动（web stage 不构建时它们变为 inert，详见交付报告）。
#   ⚠️ ACR 控制台「源码自动构建」是仓库外并行链路，本波不碰（仅登记）。
# ============================================================

# ---- web 阶段：构建 Kuikly H5 管理页静态产物（client Gradle 工程）----
# `gradle:8.14.5-jdk17-jammy`：与 client/gradle/wrapper 完全同版本（8.14.5）+ JDK17。
# 镜像已经安装同版本 Gradle，Docker 直接调用 gradle，不再经 wrapper 重复下载发行包。
# 首条任务核对镜像内 Gradle 与 wrapper 版本，版本不一致立即失败；本地仍用 ./gradlew。
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
COPY client/gearuiKit/build.gradle.kts client/gearuiKit/build.gradle.kts
COPY client/apps/h5App/build.gradle.kts client/apps/h5App/build.gradle.kts
WORKDIR /src/client
# Gradle 用户主目录显式钉到挂载路径（官方 gradle 镜像的默认值，写出来防镜像漂移）。
# 下方三条 RUN 共挂同一块命名缓存（同一路径即同一份内容）：依赖/插件/
# 构建缓存（gradle.properties 已开 org.gradle.caching）跨层失效、跨构建复用。
# Gradle 发行版本体由基础镜像提供，不依赖挂载缓存里恰好有 wrapper/dists。
ENV GRADLE_USER_HOME=/home/gradle/.gradle
# 只做配置/下载依赖，不编译业务代码（保持轻）
RUN --mount=type=cache,target=/home/gradle/.gradle \
    set -eu; \
    wrapper_version="$(sed -nE 's/^distributionUrl=.*gradle-([0-9]+(\.[0-9]+)*)-(all|bin)\.zip$/\1/p' gradle/wrapper/gradle-wrapper.properties)"; \
    installed_version="$(gradle --version | awk '/^Gradle / { print $2; exit }')"; \
    if [ -z "$wrapper_version" ] || [ "$installed_version" != "$wrapper_version" ]; then \
      echo "Gradle version mismatch: image=$installed_version wrapper=$wrapper_version; update the web base image and wrapper together" >&2; \
      exit 1; \
    fi; \
    gradle :apps:h5App:help --no-daemon

# 业务源码：shared（KMP 共享层，@Page 页面 + Kuikly 插件打业务包）+ gearuiKit + h5App 壳
# ⚠️ 上方 WORKDIR 已切到 /src/client（上面那个 help 任务的需要），COPY 的 dest 相对 WORKDIR 解析：
#    必须写 ./shared、./gearuiKit、./apps/h5App（= Gradle 根 /src/client 下的模块目录，
#    对应 settings 的 :shared、:gearuiKit、:apps:h5App）。曾误写 ./client/shared，源码实际
#    落盘 /src/client/client/shared（双层嵌套）：COPY 静默成功，Gradle 侧 shared/ 只有
#    build.gradle.kts 没有 src → js 源集 NO-SOURCE → KSP 不生成 KuiklyCoreEntry.kt →
#    打包任务深处 FileNotFoundException。
COPY client/shared ./shared
# gearuiKit：:shared 的依赖模块（GearUI 组件库，settings include :gearuiKit）。
COPY client/gearuiKit ./gearuiKit
COPY client/apps/h5App ./apps/h5App

# 🚨 源码断言：验证业务源码真的落在了 Gradle 工程期望的位置（而非嵌套错位或上下文缺目录）。
#   缺失/错位时 Gradle 会把 js 源集判成 NO-SOURCE → KSP 不执行 → KuiklyCoreEntry.kt 不生成
#   → 打包任务深处抛晦涩的 FileNotFoundException（实测部署链路 409s 才失败）。这里在进入
#   Gradle 前快速失败，错误信息直指根因。
RUN if ! (test -d shared/src/commonMain/kotlin \
          && test -f shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigPage.kt \
          && test -d gearuiKit/src/commonMain/kotlin \
          && test -f apps/h5App/src/jsMain/kotlin/com/xiaozhi/admin/Main.kt); then \
      echo "❌ web 构建缺业务源码：/src/client/{shared,gearuiKit,apps/h5App}/src 未就位（COPY 目标路径错位或上下文不完整）" >&2; \
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
RUN --mount=type=cache,target=/home/gradle/.gradle \
    gradle :shared:compileKotlinJs :shared:compileProductionExecutableKotlinJs \
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
RUN --mount=type=cache,target=/home/gradle/.gradle \
    NODE_OPTIONS="--max-old-space-size=${NODE_MAX_OLD_SPACE}" \
    gradle :apps:h5App:publishWeb --no-daemon --max-workers=${GRADLE_MAX_WORKERS} -PwebSourceMap=false \
      -Dorg.gradle.jvmargs="-Xmx${GRADLE_XMX} -Dfile.encoding=UTF-8"
# W15-1：旧端 publishWeb 的产物断言（web/index.html / nativevue2.js / h5App.js /
#   web/em/*）已随「生产镜像不再依赖旧端产物」移除。等价校验迁到下面的 admin-web stage：
#   `pnpm build` 后断言 dist/index.html + dist/em/index.html +
#   dist/em/static/{adapter.js,dashboard.js,host.js}（+ dist/logo.png）。
#   web stage 定义完整保留，仅供本地回滚重建旧端时使用。

# ---- admin-web 阶段：构建 React 管理台 SPA + /em/ 白名单资产（W15-1）----
# dist/ 不入库（client/admin-web/.gitignore），镜像必须自建；Node 22 + pnpm 12.15.0
# 与本机 / tauri.conf.json 的 beforeBuildCommand "pnpm build" 同款工具链。
# `pnpm build` 先跑 scripts/sync-em.mjs 把 em 源目录的白名单 4 文件同步到 public/em/，
# 再由 Vite 默认 publicDir 在**同一次 build 内**（清空 outDir 之后）拷进 dist/em/——
# 这是唯一与 build.emptyOutDir:true 天然兼容的方案，不需要改 vite.config.ts。
FROM node:22-alpine AS admin-web
WORKDIR /src/client/admin-web
# em 源目录（client/apps/h5App/em/）是 /em/ 官方页唯一来源，软下线时原位保留；
# 这里只带 sync-em.mjs 需要的白名单 4 文件，不带 dashboard.jsx / tests / README / LICENSE。
COPY client/apps/h5App/em/index.html /src/client/apps/h5App/em/index.html
COPY client/apps/h5App/em/static/adapter.js /src/client/apps/h5App/em/static/adapter.js
COPY client/apps/h5App/em/static/dashboard.js /src/client/apps/h5App/em/static/dashboard.js
COPY client/apps/h5App/em/static/host.js /src/client/apps/h5App/em/static/host.js
# 先只拷依赖清单，源码改动不触发依赖重装。
COPY client/admin-web/package.json client/admin-web/pnpm-lock.yaml client/admin-web/pnpm-workspace.yaml ./
RUN npm install -g pnpm@12.15.0 \
 && pnpm install --frozen-lockfile
COPY client/admin-web/tsconfig.json client/admin-web/tsconfig.app.json client/admin-web/tsconfig.node.json \
     client/admin-web/vite.config.ts client/admin-web/index.html ./
COPY client/admin-web/src ./src
COPY client/admin-web/scripts ./scripts
# .dockerignore 的 `logo.png` 会连带排除 public/logo.png，已在 .dockerignore 里开例外；
# 这里再显式指定单文件，确保 dist/index.html:7 引用的 /logo.png 一定随镜像分发。
COPY client/admin-web/public/logo.png ./public/logo.png
RUN pnpm build \
 && test -f dist/index.html \
 && test -f dist/logo.png \
 && test -f dist/em/index.html \
 && test -f dist/em/static/adapter.js \
 && test -f dist/em/static/dashboard.js \
 && test -f dist/em/static/host.js

# ---- Rust 构建阶段 ----
# rust:1.95-bookworm = 满足 WASM 宿主（wasmtime 48）所需的 rustc 版本。
# 保持 Debian（非 alpine）：sherpa-onnx 预编译原生库与 audiopus/libopus 按 glibc 链接，
# 与 xiaoya 的 musl 静态方案不通用，勿照搬。
FROM rust:1.95-bookworm AS build

# pkg-config + libopus-dev：audiopus_sys 需要（libopus 编译/链接）
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libopus-dev \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 🚨 阶段串行屏障（见文件头 2026-10-27 记录，勿删）：BuildKit 默认并行执行没有依赖关系的
#   stage。W15-1 起 admin-web（node/pnpm + Vite 构建）与 build（cargo 的 rustc 进程群，
#   本依赖树含 ort/tokenizers/sherpa-onnx，单进程 1.5–2.5GB）并行驻留仍会抬高构建机
#   峰值内存，4–8GB 档有 OOM 风险。这里读一个 admin-web 阶段产物建立 DAG 依赖：
#   admin-web 先跑完，cargo 才开始；文件落 /tmp，不进运行镜像。
#   代价：总时长 ≈ admin-web + build（原先并行取 max），换峰值内存永不同时出现。
#   ⚠️ web（Gradle）阶段仍在文件中保留定义（软下线），但已无 `COPY --from=web` 引用，
#      BuildKit 不会构建它——生产镜像不再依赖旧端产物。
COPY --from=admin-web /src/client/admin-web/dist/index.html /tmp/.web-stage-done

# 🚨 rustc 并行度上限（ACR 档默认 2，勿删）：cargo 默认 `-j=CPU 核数`，本依赖树含
#   wasmtime/cranelift / sherpa-onnx-sys（自带 C++/protobuf codegen），单 rustc 峰值 1.5–2.5GB，
#   -j4 就能吃掉 8GB 机器的全部内存。2 = 实测能过 ACR 构建机的档位（慢一点，但不会中断）；
#   GHA（16GB/4vCPU）经 build-args 传 4 对齐核数——上方串行屏障保证与 web 峰值错开，
#   4 路 rustc 并发（最坏 ~10GB）16GB 放得下。ARG 对本阶段后续 RUN 即环境变量，cargo 直读。
ARG CARGO_BUILD_JOBS=2

# 目标指令集固定为 x86-64-v2：部署机 N5105（Tremont，无 AVX）可安全运行。
# 不要用 native —— ACR 构建机的 CPU 与部署机不同，native 会嵌入部署机不支持的
# AVX/AVX2 指令，导致运行时 SIGILL 崩溃。
ENV RUSTFLAGS="-C target-cpu=x86-64-v2"

# sherpa-onnx 预编译库预取层：sherpa-onnx-sys 的 build.rs 默认从 GitHub release 资产端点
# 下载静态库归档，该端点间歇性 504/传输中途截断（2026-11-08 GHA 与本地同时复现；ureq 无
# 重试，一撞即 build.rs:42 panic）。对策：预取到 /opt/sherpa-archives 并设
# SHERPA_ONNX_ARCHIVE_DIR——build.rs 见目录内有精确文件名的归档即跳过下载（本仓库
# ~/.cargo 缓存中 sherpa-onnx-sys-1.13.8/build.rs 的既定契约）。源顺序：GitHub 直连（健康时
# 最快，GHA 美国 runner 常态首选）→ gh-proxy.com 前置代理（国内/直连故障兜底，社区代理
# 有篡改风险，靠下方 sha256 钉版校验兜住）。⚠️ 升级 sherpa-onnx-sys（Cargo.lock 版本变化）
# 后 case 未命中会明确报错：需下载新归档补一行 sha256 再构建——保「可复现 + 防篡改」。
COPY server/Cargo.lock ./
RUN SHERPA_VER="$(awk '/^name = "sherpa-onnx-sys"$/{getline; gsub(/"/, ""); print $3; exit}' Cargo.lock)" \
 && ARCHIVE="sherpa-onnx-v${SHERPA_VER}-linux-x64-static-lib.tar.bz2" \
 && case "${SHERPA_VER}" in \
      1.13.8) SHA256=e1fdc5b67530e15741ef897fa5ffff297056f3bf0c6d829a27af9225a4c4b5a6 ;; \
      *) echo "sherpa-onnx-sys ${SHERPA_VER} 未钉版：下载 ${ARCHIVE} 算 sha256 后补进本 RUN 的 case 分支" >&2; exit 64 ;; \
    esac \
 && mkdir -p /opt/sherpa-archives && cd /opt/sherpa-archives \
 && ok=0 \
 && for base in \
      "https://github.com/k2-fsa/sherpa-onnx/releases/download/v${SHERPA_VER}" \
      "https://gh-proxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download/v${SHERPA_VER}"; do \
      echo "[sherpa] 拉取 ${ARCHIVE} ← ${base}" \
      && curl -fL --connect-timeout 10 --max-time 300 --retry 2 --retry-all-errors -o "${ARCHIVE}" "${base}/${ARCHIVE}" \
      && echo "${SHA256}  ${ARCHIVE}" | sha256sum -c - \
      && ok=1 && break \
      || echo "[sherpa] 本源失败（504/截断/哈希不符），换下一源" >&2; \
    done \
 && [ "${ok}" = 1 ] \
 && ls -l /opt/sherpa-archives
ENV SHERPA_ONNX_ARCHIVE_DIR=/opt/sherpa-archives

# 依赖缓存层：先 COPY 清单 + dummy main.rs 编译全部依赖（sherpa 静态库由上方预取层供给，
# 不再现场下载）。--locked：清单与 lock 不一致即构建失败（可复现发布镜像是核心诉求）。
COPY server/Cargo.toml server/Cargo.lock ./
# registry/git 挂命名缓存（两条 cargo RUN 共挂，源码层那条也要挂——上游 crate 源码解包在
# CARGO_HOME/registry/src，源码层校验指纹时要读）：清单变更使本层失效时，crates.io 下载
# 与解包不重来。⚠️ target 产物不挂缓存、仍烙进本层——dummy 层给源码层供增量依赖是既有
# 设计（见下方源码层 RUN），产物进层它才成立。
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    mkdir -p src \
 && echo 'fn main() {}' > src/main.rs \
 && cargo build --release --no-default-features --features sherpa,nlu,wasm-plugins --locked \
 && rm -rf src

# 真实源码（bin-only crate：入口即 src/main.rs，无 lib.rs）。插件源码
# 独立放在仓库根 plugins/，由 src/main.rs 以 #[path = "../../plugins/mod.rs"] 接入同一 crate。
# ⚠️ 容器内 Cargo manifest 在 /app（对应本地 server/）、main.rs 在 /app/src，故 #[path]
#    的 ../.. 落在容器根 /：插件目录**必须** COPY 到 /plugins（crate 根的上一层，与本地
#    「plugins/ 与 server/ 平级」语义一致；与既有 /tools/nlu-trainer/train.py 同属
#    「容器根 / = 仓库根、/app = server/」这套映射）。⛔ 不能写 ./plugins（= /app/plugins，
#    少一层，rustc 报 couldn't read src/../../plugins/mod.rs；详见文件头第六次复现）。
COPY server/src ./src
COPY plugins /plugins
# WASM 宿主 bindgen! 在编译期读取 server/wit/plugin.wit；容器内
# Cargo manifest 位于 /app，因此契约必须落到 /app/wit/plugin.wit。
COPY server/wit ./wit
# 编译期内嵌资源：bench_api 样本经 include_bytes!（src/app/bench_api/../../../test_assets）
# 在编译期读 /app/test_assets/，缺了 cargo 直接编译失败（ACR 2026-10-21 实测）。
COPY server/test_assets ./test_assets
# NLU build code embeds this script at compile time; keep the source available to Cargo.
# The release compose may grant the explicitly documented host Docker socket for training.
COPY tools/nlu-trainer/train.py /tools/nlu-trainer/train.py
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
#   ④ find 清单里的 /plugins 是绝对路径，与上方 COPY 目标一致；⛔ 别改回相对 `plugins`
#      （WORKDIR=/app 下已无 /app/plugins，find 会直接报错短路，插件源码 mtime 也刷不到）。
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    find src /plugins -type f -exec touch {} + \
 && cargo clean -p xiaozhi-server-rust --release \
 && cargo build --release --no-default-features --features sherpa,nlu,wasm-plugins --locked \
 && strip target/release/xiaozhi-server-rust

# ---- 官方系统级 WASM 插件构建阶段 ----
# 系统插件不编进 server 二进制，但随发行镜像构建并由 boot 生命周期自动注册/激活。
FROM build AS system-plugins
ENV RUSTFLAGS=""
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add wasm32-unknown-unknown \
 && cargo install wasm-tools --version 1.263.0 --locked
COPY server/crates /server/crates
COPY server/wit /server/wit
COPY plugins-src /plugins-src
COPY tools/wasm-fixtures /tools/wasm-fixtures
COPY tools/build-system-plugins.sh /tools/build-system-plugins.sh
RUN chmod +x /tools/build-system-plugins.sh \
 && /tools/build-system-plugins.sh

# ---- 运行阶段 ----
FROM debian:bookworm-slim AS runtime

# 日志时间戳本地化：tzdata 提供时区库，TZ 缺省 Asia/Shanghai
#   （chrono Local 按 TZ → /usr/share/zoneinfo 解析，app::logtime 统一出口；
#   部署在其他时区时用 compose 的 environment: TZ=… 覆盖即可，不必改镜像）。
ENV TZ=Asia/Shanghai

# libopus0：运行期动态链接（audiopus）；curl/bzip2：entrypoint 缺模型时按需下载解包。
# docker CLI：NLU 模型训练由 compose 显式挂载的宿主 Docker socket 承载，只运行专用训练镜像。
# ONNX Runtime 1.22.1 动态库：nlu 特性下 ort 以 load-dynamic 运行时 dlopen，
#   只解出 Linux native 库供候选模型测试/推理使用。
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
# 同源托管的管理页静态产物（server [server].admin_dir 由下方 ENV 指到 /app/web）。
# W15-1：只来自 admin-web stage 自建的 React SPA dist/（含 /em/ 白名单 4 文件），
# 不再 COPY 旧端 Gradle publishWeb 的 web/ 产物；目录名 /app/web 与 ENV 保持不变。
COPY --from=admin-web /src/client/admin-web/dist /app/web
COPY docker-entrypoint.sh /app/docker-entrypoint.sh
# 内置配置示例：字段说明参照（配置真源为 SQLite data.db；示例不再作为首启配置生成）
COPY server/config.example.toml /app/config.example.toml
# 官方系统插件产物来自 system-plugins stage；用户插件仍只从数据卷/管理端上传。
COPY --from=system-plugins /build/system-plugins /app/system-plugins
RUN chmod +x /app/docker-entrypoint.sh \
 && test -x /app/server \
 && test -s /app/system-plugins/weather.official.wasm \
 && test -s /app/system-plugins/hello.official.wasm

# 数据卷：模型持久化挂载点（entrypoint 缺失时自动下载到此；必须可写，勿 :ro 挂载）
RUN mkdir -p /data
VOLUME ["/data"]

# 运行镜像内置默认值——只放「容器路径」类必须项，避免与代码默认值漂移：
#   XIAOZHI_CONFIG_DB：配置库文件（/data 内；空库首启以内置默认启动并落库）
#   XIAOZHI_ADMIN_DIR：管理页静态目录覆盖（见 main.rs load_config），指向上方 /app/web
#   XIAOZHI_AUTO_DOWNLOAD_MODELS：missing(默认,缺失才下) | force | off
#   （SenseVoice/Kokoro/Silero VAD 从 sherpa-onnx Release 下载；SENSEVOICE_URL/
#   KOKORO_URL/SILERO_VAD_URL 可覆盖为内网镜像地址。）
# 注意：镜像不内置 RUST_LOG 默认值（保持代码单一默认源），需要调业务级别时在 compose
#   的 environment 里按需设置 RUST_LOG。
ENV XIAOZHI_CONFIG_DB=/data/data.db \
    XIAOZHI_ADMIN_DIR=/app/web \
    XIAOZHI_AUTO_DOWNLOAD_MODELS=missing \
    ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so \
    XIAOZHI_SYSTEM_PLUGINS_DIR=/app/system-plugins
# host 网络部署（docker-compose 默认）下 EXPOSE 仅作文档，监听端口即宿主端口。
EXPOSE 8000

# 健康检查：curl 已随运行层安装；/api/health 不依赖模型就绪，start-period 20s 足够。
# 端口读服务启动时落盘的 /data/.runtime_port（见 main.rs），改 [server].port 无需同步这里；
# 文件缺失（如旧版镜像的挂载卷）时兜底默认 8000。
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -f http://127.0.0.1:$(cat /data/.runtime_port 2>/dev/null || echo 8000)/api/health || exit 1

ENTRYPOINT ["/app/docker-entrypoint.sh"]
