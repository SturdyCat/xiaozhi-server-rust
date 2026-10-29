# T13 四端验收清单（GearUI 迁移收口）

> 本文档汇总 GearUI 迁移（T01–T12）各轮报告中的验收项与「需本地验证」项，作为 T13 的**唯一执行清单**：
> 自动验收照 §1 逐条跑，本地验证照 §2 逐条勾。验收时不算 bug 的已知差异见 §3，回滚策略见 §4。
>
> **数字口径**：文中护栏计数 451/48/0 以「A 完成测试台重写（T12 主体）后，用 codemod `--check`
> 实测回填进 `shared/build.gradle.kts` 的常量为准」。**常量回填之前跑六件套必然红**——先等回填，再执行本清单。
>
> 所有 gradle 命令的工作目录均为 `client/`，统一加 `--console=plain`；涉 Node 的任务加 `env -u NODE_OPTIONS`；
> Android 编译需 `ANDROID_HOME="$HOME/Library/Android/sdk"`。

---

## 一、自动验收（CI 可跑）

### 1.1 六件套（一次命令，全绿 = BUILD SUCCESSFUL）

```bash
cd client && ANDROID_HOME="$HOME/Library/Android/sdk" env -u NODE_OPTIONS ./gradlew \
  :shared:compileKotlinMacosArm64 \
  :shared:compileKotlinJs \
  :shared:compileDebugKotlinAndroid \
  :shared:verifyAdminIcons \
  :shared:verifyComposeStateMigration \
  :adminCoreTests:jvmTest --console=plain
```

| # | 项 | 期望输出 |
|---|---|---|
| 1 | `:shared:compileKotlinMacosArm64` | BUILD SUCCESSFUL |
| 2 | `:shared:compileKotlinJs` | BUILD SUCCESSFUL |
| 3 | `:shared:compileDebugKotlinAndroid` | BUILD SUCCESSFUL |
| 4 | `:shared:verifyAdminIcons` | `Verified 26 embedded vector icons: Canvas rendering, no resource/bitmap fallback` |
| 5 | `:shared:verifyComposeStateMigration` | `Compose state migration: stateVar=451 (expect 451), stateListOf=48 (expect 48), legacy=0 (expect 0)`，且 **`Legacy bridge whitelist = []`**（白名单必须已清空；非空 = T12 末检未做） |
| 6 | `:adminCoreTests:jvmTest` | 90 tests / 0 failures（以 `adminCoreTests/build/test-results/jvmTest/*.xml` 汇总为准；该模块按文件白名单编译纯契约源码） |

> 护栏计数沿革（防误判"调松"）：T05=483 → T06=482（ToastHost 状态交给 GearUI Overlay）→
> R1 削减=455（删 4 页 + 3 个 state 类）→ T12 测试台重写=451（A 实测回填，死代码删除的回填）。
> 每次回填的原因都写在 `shared/build.gradle.kts` 计数注释里。

**附加回归**（client/AGENTS.md 要求的 BLE 协议回归，不在六件套内但必须跑）：

```bash
cd client && env -u NODE_OPTIONS ./gradlew :bleProtocolTests:jvmTest --console=plain
# 期望：15 tests / 0 failures
```

### 1.2 iOS 两条 compile

```bash
cd client && env -u NODE_OPTIONS ./gradlew \
  :shared:compileKotlinIosArm64 :shared:compileKotlinIosSimulatorArm64 --console=plain
# 期望：BUILD SUCCESSFUL（此前实测 9s–13s 量级）
```

### 1.3 macOS framework 链接（⚠️ 需先清 `~/.konan` 缓存锁）

T06/T10 期间 `linkDebugFrameworkMacosArm64` 一直被**本机对 `~/.konan`（工作区外）的删除权限策略**挡住，
不是代码问题（同一次构建里 `compileKotlinMacosArm64` UP-TO-DATE 即绿）。本地执行时**先清一次被中断
构建留下的 stale lock 与 `-cacheNNN` 临时目录**，再跑链接：

```bash
# 仅清理 Kotlin/Native 构建缓存（不影响源码与 Gradle 缓存），需手动在本机终端执行：
rm -rf ~/.konan/kotlin-native-prebuilt-macos-aarch64-2.1.21/klib/cache/macos_arm64-gSTATIC-pl/com.tencent.kuikly-open:core

cd client && env -u NODE_OPTIONS ./gradlew :shared:linkDebugFrameworkMacosArm64 --console=plain
# 期望：BUILD SUCCESSFUL；随后 scripts/patch_framework_macabi.py 的 finalizedBy 自动补丁（幂等）
```

> 该命令由验收人**在本机终端手动执行**（涉及用户家目录删除，沙箱/CI 不代办）。arm64 编译本身
> 已由 1.1 之 #1 覆盖，此处只验链接产物 + macabi 补丁链。

### 1.4 Android 单元测试（StateVarTest）

```bash
cd client && ANDROID_HOME="$HOME/Library/Android/sdk" env -u NODE_OPTIONS ./gradlew :shared:testDebugUnitTest --console=plain
# 期望：5 tests / 0 failures / 0 skipped（StateVarTest 的 5 个纯逻辑用例）
```

### 1.5 web 打包 + 产物抽验

```bash
cd client && env -u NODE_OPTIONS ./gradlew :apps:h5App:publishWeb --console=plain
# 期望：BUILD SUCCESSFUL，产物更新（client/apps/h5App/web/nativevue2.js 约 3.9MB 量级 / h5App.js）

# 产物抽验（在 client/apps/h5App/web 下）：
grep -c "批准并命名" nativevue2.js   # 期望 ≥1（EchoMuse 审批 sheet 文案；上轮实测 =1）
grep -c "跟随中"     nativevue2.js   # 期望 ≥1（控制台跟随开关文案；上轮实测 =1）
grep -c "上传固件"   nativevue2.js   # 期望 ≥1（OTA 上传按钮文案）
grep -c "opsHeader"  nativevue2.js   # 期望 =0（旧 DSL 运维头组件零残留）
grep -c "admin-compose" nativevue2.js # 期望 =0（已删的 @Page("admin-compose") 试点入口零残留）
```

> 文案计数以 `grep -c` 实跑为准；`批准并命名`/`跟随中` 在最近一次打包轮实测均为 1，
> 其余按"存在 ≥1 / 禁止残留 =0"口径判定。

### 1.6 全仓静态断言（⚠️ 前提：T12 清理收口后）

**前提**：`ui/` 14 个文件、`LegacyVfor.kt`、`icons/LegacyAppIcon.kt` 均已删除（清理收口前跑必红，
当前仓库这三个命中仍存在）。收口后执行：

```bash
# ① 传统 DSL 原语归零（期望：空集，无任何输出）
grep -rlE "vfor\(|ViewContainer<" --include=*.kt client/shared/src

# ② 自研 token 零引用（-E 扩展正则 + 排除注释行；期望：空集）
grep -rlE "AdminColors|AdminType|AdminToken|AdminShape" --include=*.kt client/shared/src \
  | xargs -I{} grep -lE "AdminColors|AdminType|AdminToken|AdminShape" {} \
  | while read f; do grep -nE "AdminColors|AdminType|AdminToken|AdminShape" "$f" \
      | grep -vE ":[[:space:]]*(\*|//)" | grep -q . && echo "$f"; done
```

> ② 的排除注释口径是必须的：`kit/GearTerminal.kt:32`、`icons/AdminIcons.kt:145`、`EchoMuseParts.kt:36`
> 的命中全是注释里的历史说明文字（已逐行核实），代码引用为零。
> `LegacyAppIcon.kt` 是**唯一**带真实 `AdminColors` 代码引用的豁免文件（T12 一并删除）。

### 1.7 KSP 页面注册收敛

三端生成物各只含两条注册（`admin` + `config`），多一个即红：

```bash
grep -c "registerPageRouter" client/shared/build/build/generated/ksp/{macosArm64,js,androidMain}/macosArm64/com/xiaozhi/admin/KuiklyCoreEntry.kt 2>/dev/null
# 实际路径以三端各自 KSP 输出目录为准；每端期望恰好 2 条：
#   registerPageRouter("config"){ConfigPage()} 与 registerPageRouter("admin"){AdminPage()}
```

---

## 二、本地验证清单（用户执行，逐项勾选）

### 2.1 整站（web）

- [ ] 起静态服务巡检：`cd client/apps/h5App/web && python3 -m http.server 8080`，浏览器开 `http://localhost:8080`
- [ ] 浅色主题巡检：GearUI 默认浅色（D2 决策），整站无深色残留、无主题混排
- [ ] 品牌绿仅剩侧栏选中态一处，页面正文不再出现品牌绿装饰
- [ ] 8 项导航全部可达且切换正常：概览 / 设备 / 语音流程 / 全局配置 / EchoMuse / 控制台 / 文件 / 测试台（mac only，web 不可见）
- [ ] 26 个图标的运行时视觉确认（T04 起不做截图对比，代码级等价已由 verifyAdminIcons 保证，视觉留给本项）
- [ ] 窄窗响应式：768 / 900 / 700 / 240 断点下的分栏与侧栏行为

### 2.2 语音流程（配置页）

- [ ] 轨道 Steps 渲染正常（PipelineWorkflowView）
- [ ] 双 Tab 切换正常（轨道图 / 共享节点测试）
- [ ] 10 个 section 表单回显正确（拉取 → 回填 → 修改 → 保存 → 重进回显一致）
- [ ] 节点测试 Result 展示正确（结果 / 延迟 / 错误三态）

### 2.3 全局配置（配置页）

- [ ] 健康横幅四色：error 红 / busy 黄 / connected 绿 / idle 灰；**正常行不打标**（中性文字「正常」，不渲染绿胶囊——旧实现刻意规则）
- [ ] presets 在设备 overlay 模式下禁用：editDeviceId 非空时只显示禁用说明，不渲染表单
- [ ] MCP 卡只读投影：无保存语义，空列表 / 断开态文案正常
- [ ] OTA 固件上传链路：选文件 → 上传 → **不确定态进度条**（往复扫动，无假百分比）→ 成功刷新列表
- [ ] OTA 删除：`ConfirmDialog(destructive=true)` 二次确认，误触外部不关闭

### 2.4 EchoMuse（设备详情）

- [ ] 六 tab `Tabs(scrollable=true)`：窄窗横向滚动手感正常，不折叠"更多"
- [ ] 深色终端块：GearTerminal 配色（`#1C1F18` 底 `#C8D4B0` 字），ANSI 日志可读
- [ ] 设备删除二次确认（ConfirmDialog destructive）
- [ ] 「批准并命名…」sheet：未批准设备审批区 → 弹层命名 → 批准
- [ ] Meter 资源条越界变色：≥75% 黄、≥90% 红（渲染层阈值）；量程缺失时退回缺测 tile、不崩
- [ ] 唤醒词自定义模型入口：跳「文件」页 oww-models/ 对应目录

### 2.5 控制台 / 文件页

- [ ] 控制台跟随尾行：顶栏「跟随中 / 已暂停」**显式开关**（默认跟随；暂停后新输出不拽人，再点恢复滚到尾行）
- [ ] 文件页面包屑跳转、上传、新建、删除（删除走 ConfirmDialog）
- [ ] 极窄宽度下文件页三按钮 + 面包屑不溢出（`actionRow` 不换行是已知差异，见 §3）

### 2.6 测试台（macOS App）

- [ ] **⚠️ spike 唯一遗留风险先行确认**：原生宿主对 `playRecording` / `playTts` / `stopPlayback` 三方法仍健在（宿主代码不在本仓库；若宿主当年随 AudioView 升级裁掉了，需宿主侧补 AVPlayer 直控，量很小。`audioFileUrl` 保留不删，作宿主侧排查对照）
- [ ] mDNS 服务器切换：侧边栏「服务器设置」入口（**前提：T12 接线恢复该入口**，此前 `serverSwitchOpen` 保留未接）→ 自动发现列表出服务器 → 切换后 WS/API 同步换基址
- [ ] 五 tab 全流程：连接 → ASR 识别（录音→计秒→试听波形→发送识别）→ TTS 合成（音色/语速，可临时覆盖）→ LLM 对话 → NLU 意图
- [ ] 识别 / 回复文本复制按钮（macOS 走真实现；Android 上点击静默无操作为既有行为，见 §3）
- [ ] 播放控制走模块层直控（方案 Y）：播放中可 stopPlayback，不出现双声 / 卡死

### 2.7 macOS framework + 宿主

- [ ] §1.3 清缓存后 `linkDebugFrameworkMacosArm64` 通过
- [ ] `xcodegen generate` → `pod install` → Xcode 开 workspace，运行目标 **My Mac (Mac Catalyst)**
- [ ] 启动白屏检查：白屏（页面回退占位）先 Clean Build Folder——Kuikly 找不到 page 的 fallback 老坑，不是代码 bug
- [ ] 改过 `@Page` 或 shared 业务代码后重跑 `:shared:linkReleaseFrameworkMacosArm64`，确认 Xcode 跑的是新框架

---

## 三、已知登记的差异（验收时不算 bug）

| # | 差异 | 原因 / 出处 |
|---|---|---|
| 1 | 控制台跟随尾行为「显式开关」而非旧版"离底距离自动开关 autoScroll" | 本 Kuikly 2.30.0 版本无 `snapshotFlow`、无等价 scroll 回调（klib 探针确认）；`console.autoScroll` 字段与签名零改动，差异已写文件头注释 |
| 2 | BLE 广播 / 属性清单随页面流排版（不再内嵌 240/320 定高 Scroller） | T10 登记的 A 类版式差异 |
| 3 | `actionRow` 是 `Row` 不换行 | kit 已登记的 A 类差异；文件页三按钮 + 面包屑在极窄宽度可能溢出 |
| 4 | `TriggerDialog` 默认焦点由 GearUI 组件决定，无法强制"默认落在取消" | 已用 `destructive = true` + `dismissOnOutside = false` 兜住不可撤销动作的误触 |
| 5 | Android 复制按钮静默无操作 | `copyToClipboard` androidMain 为 stub 恒返 false（既有行为）；macOS/iOS/web 均真实现。T12 只标注不扩 scope |
| 6 | 护栏计数 455 → 451 | T12 测试台重写删除死代码后的 `--check` 实测回填（同 R1 的 482→455 性质：回填，不是调松） |
| 7 | OTA 上传进度为不确定态进度条，无字节级百分比 | 状态层无字节进度字段且禁改；不显示假百分比（team-lead 已认可） |

---

## 四、回滚点说明

**无单点回滚。** D3（一次性全量替换）的必然代价：T04 之后任何中间态都不保证可独立回退，
出问题时只能**整支回退**（架构文档 §6 风险 R-A 已登记）。

- 仓库当前**没有任何 git tag**。T01（Kuikly 升级）、T06（壳层跑通）等关键中间点当时应打未打——
  **已过时，不建议补打**：这些中间态是"迁移途中"的半成品（ui/ 与 kit/ 并存、编译绿但页面混杂），
  回退到它们没有任何可用价值，且构建环境（Konan 缓存、yarn.lock）已向前演化。
- 唯一有意义的回滚基线是**迁移起点**：`git log` 里 T01 之前的最后一个提交（客户端仍在 Kuikly DSL
  老体系上完整工作）。若 T13 验收失败且问题面过大，回退目标就是这个点，而不是任何中间点。
- 建议：T13 全绿后立即打一枚验收基线 tag（如 `gearui-migration-accepted`），作为此后所有演进的
  已知良好点。
