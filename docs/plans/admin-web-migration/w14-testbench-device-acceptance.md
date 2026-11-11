# W14 Testbench 真机验收清单（macOS）

> ⚠️ **编译/mock 通过 ≠ 真机硬件通过**（`src-tauri/src/commands/mod.rs:11` 与架构文档 §3-L3）。
> 本清单把编译/mock 层面已验证的功能逐条展开为**可执行的真机验收步骤**：每项写
> 「验证什么 / 怎么验 / 预期 / 失败时怎么办」。全部通过前，测试台页面的
> 「接口可用 ≠ 真机通过」徽标语义保持成立。

权威依据：`docs/plans/admin-web-migration/w14-testbench-architecture.md`（架构路径 X：
本地 loopback WS 桥只承载音频 I/O，协议编排在前端唯一 `/api/ws` 会话）；
旧端锚点 `XiaoZhiModule.m`（音频/协议）/ `+Opus.m:128-161`（pre-skip 坑）/
`TestBenchState.kt`（状态机与超时）；服务端 `bench.rs` / `bench/synth.rs` / `downlink`。

---

## 1. WebView LNA（Local Network Access）是否拦 127.0.0.1 WS —— **头号风险**

- **验证什么**：macOS 26 Chrome 系引入的 Local Network Access 询问策略，WKWebView
  是否跟进对 `ws://127.0.0.1:<port>/bridge` 的升级请求弹出「允许本地网络」或直接拦截。
  架构文档把这一项列为路径 X 的最大不确定点——WKWebView 行为只能真机观察。
- **怎么验**：
  1. `cargo tauri dev`（或打包后启动）进入测试台页；
  2. 点「ASR 识别」→「开始录音」；观察是否弹系统级「"App"想要连接本地网络」询问；
  3. 打开 Console.app 过滤 webview 进程，查 LNA 拒绝日志（`network extension` /
     `Local Network` 关键词）。
- **预期**：升级请求成功（桥 WS `onOpen` 触发，页面进入「录音中」）或弹出一次询问、
  允许后成功且选择被记住。
- **失败时怎么办**：若被静默拦截（无询问、close code 1006）——降级路径备选：
  ① Info.plist 补 `NSLocalNetworkUsageDescription` 引导授权；② 退回架构文档
  §1.1 的 IPC 白名单方案（capability 增量需重新评审）；把实测现象（含系统版本）
  记录回架构文档 §3-L3。

## 2. macOS 15 ATS + `NSAllowsLocalNetworking` 放行效果

- **验证什么**：`Info.plist` 的 `NSAllowsLocalNetworking=true`（W14b-1 已加，经
  `bundle.macOS.infoPlist` merge）是否让 WKWebView 允许非 https 的本地 WS 升级。
- **怎么验**：
  1. Release 打包（`pnpm tauri build`）后启动（ATS 只对 release 生效，dev 不拦）；
  2. 测试台 →「开始录音」；若第 1 项已证实 LNA 不拦，此项重点看 ATS 层；
  3. `codesign -d --entitlements - <app>` 确认 entitlements 已嵌入；
  `plutil -p <app>/Contents/Info.plist | grep -i AllowsLocal` 确认键已 merge。
- **预期**：`ws://127.0.0.1:...` 升级成功，无 ATS 拦截日志（Console 过滤 `App Transport`）。
- **失败时怎么办**：确认 `tauri.conf.json` 的 `bundle.macOS.infoPlist` 路径正确、
  打包产物 Info.plist 确实含键；若键在但仍拦，查是否是 NSAppTransportSecurity
  整体字典被其它 merge 覆盖；必要时改用 `NSExceptionDomains` 显式列 loopback。

## 3. cpal CoreAudio 全双工稳定性（采集 + 播放同时开）

- **验证什么**：录音（上行）与 TTS 播放（下行）同时活跃时 cpal 双流不冲突、不爆音、
  不掉线（W14b-1/W14b-2 均用 default_host 的独立输入/输出流）。
- **怎么验**：
  1. 测试台连接 → ASR tab「开始录音」不停止；
  2. 切 TTS tab 发起合成（下行播放与上行采集并行 ~10s）；
  3. 停止录音 →「发送识别」看识别文本是否完整；回听录音看有无周期性爆音；
  4. 反复 5 轮（含 AirPods 蓝牙设备切换场景——CoreAudio 采样率漂移高发）。
- **预期**：识别文本完整；回听无爆音；桥 WS 不断开；无 `device_start_failed` 事件。
- **失败时怎么办**：若蓝牙设备下复现爆音，优先确认设备聚合采样率（Audio MIDI 设置）；
  若 cpal 流 drop 引发崩，检查 AudioManager 的 reset 时序（audio lock 与 stream drop
  顺序）；把设备型号 + macOS 版本记录归档。

## 4. 麦克风 TCC 弹窗与拒绝路径

- **验证什么**：首次录音触发 TCC「麦克风」授权弹窗；拒绝后的错误呈现路径
  （Rust 侧无法可靠区分 TCC 拒绝与设备缺失，统一按 `device_unavailable` 呈现——已登记取舍）。
- **怎么验**：
  1. 重置授权：`tccutil reset Microphone <bundle-id>`；
  2. 测试台「开始录音」→ 应弹授权框 → 允许 → 录音正常；
  3. 再次重置 → 拒绝 → 观察页面文案（预期「录音启动失败：录音设备启动失败（检查输入设备）」）；
  4. 系统设置手动开启后重试 → 恢复正常。
- **预期**：弹窗出现一次；拒绝后页面给出可行动文案（不伪装成功、不悬挂在「正在启动…」）。
- **失败时怎么办**：若拒绝后 Rust 侧返回的是 enum/config 错误而非 device_unavailable，
  补错误码映射（audio.rs `error_message_for_code`）；若弹窗不出现，检查
  bundle-id 与 Info.plist `NSMicrophoneUsageDescription`。

## 5. audiopus_sys 链接可复现性（pkg-config + libopus）

- **验证什么**：本机无 cmake 但有 pkg-config + homebrew libopus 1.6.1 的链接路径，
  在 CI / 同事机器上是否可复现（audiopus_sys 0.1.8 走 pkg-config）。
- **怎么验**：
  1. 干净环境（或 CI）执行 `cargo build -p xiaozhi-admin-web`（server 同栈依赖已验证
     可编译，admin-web 侧独立确认）；
  2. `brew list opus` 确认 libopus 在位；`pkg-config --modversion opus` 确认可发现；
  3. 产物启动后测试台完整跑一轮 ASR + TTS（编解码真跑通而非仅链接通过）。
- **预期**：编译 Exit 0；运行期 Opus 往返正常（识别文本完整、合成有声）。
- **失败时怎么办**：失败机装 `opus`（brew）或补 cmake；在 Cargo.toml 注释（已写）
  基础上补充 README 级说明；长期可考虑 vendored feature 切换，登记后另开任务。

## 6. unwrap_uplink / wrap_downlink 全链路（v1 裸帧双向）

- **验证什么**：hello `version:1` 下，上行（Rust 编码 → 前端中转 → server
  `unwrap_uplink(V1)` → ASR）与下行（server `wrap_downlink(V1)` 裸帧 → 前端中转 →
  Rust 解码播放）双方向 Opus 均被正确解出。单测只固化了 V1 identity 语义，
  端到端字节流需真机连 server 验证。
- **怎么验**：
  1. 真机连真实 server（dev 探针确认非 dev 模式）；
  2. ASR：录音 3-5 秒中文 → 停止 → 发送识别 → 识别文本与语音内容一致；
  3. TTS：合成一句 → 本地播放内容与文字一致、无周期性杂音（杂音=帧边界错位征兆）；
  4. server 日志确认无「解码失败/unknown variant」类 warn。
- **预期**：双方向内容正确。
- **失败时怎么办**：上行识别为空 → 抓 server `test_buf` 长度日志确认字节到达；
  下行无声/杂音 → 核对 server hello 的 audio_params（下行率）与 Rust
  `tts_begin` 收到的 sample_rate 是否一致（不一致 = 前端 hello 解析问题）。

## 7. 端到端延迟与波形体验

- **验证什么**：真机主观体验——ASR 识别往返耗时、TTS 首声延迟（TTFA）、
  波形推进/回放进度流畅度、回听与实播一致性。
- **怎么验**：
  1. ASR：说一句话到识别结果出现的体感耗时（页面展示本端测量的 ms 数）；
  2. TTS：点合成到出声的体感延迟；播放中波形进度点亮是否平滑（100ms play_state 推送）；
  3. 播完点「试听」回放对比实播。
- **预期**：ASR 往返秒级内；TTS 首声 < 2s（服务端 TTFA 主导）；进度平滑无跳变。
- **失败时怎么办**：延迟异常先分界——server 日志 TTFA/合成耗时 vs 本端测量；
  进度跳变查 play_state 推送间隔（Rust 100ms 节流）与前端渲染。

## 8. init script 双 origin 注入（dev 5173 / 生产 tauri://localhost）

- **验证什么**：`bridge_init_script` 的 origin 守卫在两种窗口 origin 下都正确注入
  `__XIAOZHI_BRIDGE_PORT__`/`__XIAOZHI_BRIDGE_TOKEN__`，且在其它 origin（如生产
  服务器页面直接访问桥）不注入。
- **怎么验**：
  1. dev：`cargo tauri dev`（origin = http://127.0.0.1:5173）→ 测试台录音/合成可用；
  2. Release 内嵌（origin = tauri://localhost）→ 同样可用；
  3. External 部署形态（生产服务器页面）→ 可用，且从其它站点控制台调
     `window.__XIAOZHI_BRIDGE_PORT__` 应为 undefined（origin 守卫生效）；
  4. 用 curl 直连桥端口（不带 token）→ 403（三层防线兜底）。
- **⚠️ 远程形态 LNA 点名（架构文档 §1.1① 头号风险的补充面）**：页面 origin 为
  `http://<LAN-IP>:8000`（远程服务器页面形态）时，是「**LAN 页面 → 连 ws://127.0.0.1**」
  的组合——与本地页面的 loopback→loopback 不同，Chrome LNA 对**跨源到 loopback** 的
  请求策略更严（私网→localhost 方向可能触发独立询问或静默拦截），WKWebView 是否跟进
  **必须以该形态真机单独验证**，不能以第 1 项的本地形态结论外推。
  - 验证步骤：同第 1 项，但窗口地址用 `http://<LAN-IP>:8000` 的测试台页面；
  - 预期：升级成功或一次询问后成功；
  - 失败时怎么办：若该形态被拦而本地形态放行——降级顺序：① 文档化「远程页面形态
    不支持本地音频桥」（测试台 ASR/TTS 给可行动提示）；② 评估桥凭据经页面服务器
    页面转发之外的替代通道（架构变更，需重新评审，勿自行实施）。
- **预期**：两种合法 origin 均注入生效；非法 origin 拿不到凭据；无凭据连接被拒。
- **失败时怎么办**：dev 不注入 → 检查 `allowed_origins_for_window`（devUrl 与
  conf 的 devUrl 是否一致）；External 不注入 → 检查页面 URL origin 推导；
  全部不注入 → 看 setup 里 `start_bridge` 是否 Err 降级（eprintln 有日志）。

## 9. W14a dev 模式不可用确认项（沿 W14a 交付，真机复核）

- **验证什么**：dev 模式（`dev.rs` guard）下 `/api/ws` 握手被 401/503 拦时，
  页面透出 guard 的 JSON 原文而非通用网络错误；真机（非 dev）不受影响。
- **怎么验**：
  1. dev 模式 + 错误 token 连接 → 页面应显示 `{"error":"unauthorized"}` 原文口径；
  2. dev 模式 + 正确 token → 显示 dev_inference_disabled 的 503 JSON 原文；
  3. 真 server（非 dev）→ 正常连接，无探针副作用。
- **预期**：三场景文案分别符合 `probeTestbenchHandshakeDetail` 注释所述。
- **失败时怎么办**：探针取不到原文（非 JSON 400）按 null 回退通用文案——若 dev 下
  连回退都异常，检查 dev guard 拦截顺序（先于 WS 路由）是否被改动。

## 10. 桥端口冲突、多实例与 app 退出后端口释放（架构文档 §3 第 5 项）

- **验证什么**：本地桥 `bind(127.0.0.1:0)`（OS 分配空闲端口）在端口稀缺/多实例/
  退出释放三种场景下的行为——固定端口必冲突的设计假设是否成立、同机双实例
  是否互不干扰、进程退出后端口是否立即归还 OS。
- **怎么验**：
  1. **多实例**：同时启动两个 app 实例（`open -n <app>`）→ 各自测试台录音/合成
     均可用（两实例端口不同、token 不同，互不串扰）；
  2. **端口释放**：正常退出 app（Cmd+Q）→ `lsof -iTCP -sTCP:LISTEN | grep <旧端口>`
     确认监听已消失；立即重新启动 → 新端口分配成功、测试台可用（排除 TIME_WAIT
     挂死——SO_REUSEADDR 由 tokio/axum 默认路径处理，实测确认）；
  3. **异常退出**：kill -9 实例 → 重启后端口分配正常（OS 回收，无遗留监听）；
  4. **桥启动失败降级**：极端环境（如沙箱禁网）下 `start_bridge` 失败 → 页面
     测试台 ASR/TTS 给「本地音频桥未就绪（需在 Tauri 桌面壳内运行；请重启应用）」
     提示，不崩 app（诚实降级路径）。
- **预期**：双实例共存互不影响；退出/被杀后端口即时释放可复用；桥不可用时
  功能诚实降级、主功能不受影响。
- **失败时怎么办**：端口未释放 → 排查 runtime OnceLock 常驻与 tokio runtime
  生命周期（进程退出应整体回收）；多实例互踩（A 实例连到 B 的桥）→ token 每
  launch 随机，理论上不可能互认，复现则查 init script 注入时机与窗口复用；
  记录实测现象回架构文档 §3。

---

## 验收记录

| # | 项 | 结论 | macOS 版本 / 设备 | 备注 |
|---|---|---|---|---|
| 1 | LNA | 待验 | | |
| 2 | ATS | 待验 | | |
| 3 | cpal 全双工 | 待验 | | |
| 4 | TCC | 待验 | | |
| 5 | audiopus 链接 | 待验 | | |
| 6 | v1 全链路 | 待验 | | |
| 7 | 延迟体验 | 待验 | | |
| 8 | init script（含远程形态 LNA 点名） | 待验 | | |
| 9 | dev 探针 | 待验 | | |
| 10 | 端口冲突/多实例/退出释放 | 待验 | | |
