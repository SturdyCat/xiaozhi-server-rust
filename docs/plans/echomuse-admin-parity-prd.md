# EchoMuse 设备管理界面官方对齐 PRD

> 状态：待评审 ｜ 作者：许清楚（产品） ｜ 版本锚点：官方 `controller/static/dashboard.jsx` @ HEAD `e2f2f4e`、设备 `ConfigMessage` @ `device/internal/config/config.go`
> 本文件只描述**要什么**与**边界**，不含实现代码。所有"官方行为"均可在 `/tmp/upstream/` 快照内按行号核对；所有"我方现状"均以 `server/src/device/echomuse/` 与 `client/shared/.../admin/` 源码与 `docs/echomuse.md` 为准。

---

## 0. 一句话摘要

把 EchoMuse 设备详情从"左菜单七项"重排为官方 dashboard 的**六 tab 布局**（状态 / 活动 / 配置 / 控制台 / 更新 / 日志），并补上唯一真正缺失的核心能力——**设备控制台**（浏览器里输命令、看输出的 root shell），其余缺失项按"能真实拿到数据才做、拿不到就如实显示未知"的原则补齐或明确标注不做。

一期最小闭环（6 条）：

1. 六 tab 骨架 + 未批准设备单页审批（含官方 Action-required 横幅），左菜单七项按迁移表拆解归位；
2. 控制台：Kuikly 行模式终端（WS 通道 + 会话元信息 + 历史命令 + Ctrl-C + 清屏 + 复制 + 空闲超时 + 单会话互斥 + 审计）；
3. 状态 tab：设备信息行 + 资源三条（CPU/RAM/存储）+ 三个指标位（Link/Latency/Temp，无数据显 `—`）；
4. 配置 tab：官方 7 分组骨架（Playback/Wake word/Microphones/Ring/Advanced/Bluetooth/Sendspin），只渲染现有 `SETTINGS_KEYS` 白名单里真实存在的键；
5. 更新 tab：**只读版本 + 唤醒词/VAD 资产安装状态 + 维护（重启/诊断导出）**，明示不做 OTA；
6. 日志 tab：保留现有六源白名单，改为官方行式渲染（时间 / 级别 / 来源 / 正文）。

---

## 1. 项目信息

| 项 | 内容 |
|---|---|
| Language | 简体中文 |
| Programming Language | Kotlin（Kuikly 多端，`client/shared`）+ Rust（axum，`server`）；**不是** Vite + React + MUI，沿用现有技术栈 |
| Project Name | `echomuse_admin_parity` |
| 交付物 | 本 PRD（唯一产物，不改任何源码） |
| 唯一视觉与功能来源 | 官方 `controller/static/dashboard.jsx`（11658 行）、`controller/em_controller.py`、`device/internal/config/config.go`；不凭记忆，逐条对照行号 |

### 原始需求复述

用户是 `xiaozhi-server-rust` 作者，服务端用 Rust + Kuikly 多端管理台。他抱怨 EchoMuse（Echo Dot 2 / emOS 设备）的管理界面"UI 不好用、核心功能缺失"，要求**完全照着官方 controller 的设备管理页面抄、布局对齐**。特别强调官方"点设备进去后超级好用的控制台"（shell 的 Web 版，可直接在网页里输入命令看输出）我方没有。三个既定决策：

1. "不可用" = UI 不好用 + 核心功能缺失 → **照官方抄，布局对齐**；
2. 控制台执行模式 → **对齐官方（WebSocket 终端，不是单条 HTTP 命令）**；
3. 范围 → **补齐并重排为官方六 tab 布局**。

---

## 2. 产品定义

### 2.1 产品目标（三个正交目标）

| # | 目标 | 说明 |
|---|---|---|
| G1 | **一次点击到达控制台** | 设备详情默认落 status；控制台是六 tab 之一，点一次即进；进入即可输入，不需要任何额外开关或弹窗 |
| G2 | **信息架构与官方同构** | 六 tab 的名字、顺序、归属（哪个区块在哪个 tab、哪个 tab 顶部放 WiFi）与官方一致；在官方 dashboard 学会的动作，在我方能原样找到 |
| G3 | **缺口可读、不假填** | 官方有但我方无数据的字段一律显示"未知/—/不可用 + 原因"，绝不填 0、绝不把"已下发/已排队"写成"设备已执行" |

### 2.2 成功判据

| 判据 | 量化口径 |
|---|---|
| 到达并执行命令 | 从设备列表点进设备 → 点"控制台" → 输入 `uname -a` → 看到输出，**≤ 3 次交互、0 次二次确认弹窗**，局域网端到端 ≤ 2s |
| 布局对齐 | 官方六 tab 的每个区块都能在 PRD 第 3 章"落位说明"列找到我方对应位置；无区块丢失、无"孤儿"功能 |
| 诚实度 | 抽查 10 个官方字段：我方要么有真实数据、要么显示未知+原因；0 处假填 |
| 安全 | 100% 的控制台会话在服务端留审计行（时间/设备/来源 IP/命令行）；非 approved / 非 emOS / 离线 / 未认证 / 已被占用五种情况全部被拒且文案明确 |

### 2.3 用户故事

| # | 角色 | 故事 |
|---|---|---|
| US-1 | 单机自用房主 | 作为自用房主，我想在网页里直接 `vi /data/local/etc/echomuse/config.json` 看一眼设备配置并改掉，这样我不用插数据线、不用开串口 console |
| US-2 | 单机自用房主 | 作为自用房主，我想一眼看到设备在不在、WiFi 信号多差、内存还剩多少、有没有过热降频，这样我能判断"是设备的问题还是服务器的问题" |
| US-3 | 排障中的开发者 | 作为排障中的开发者，我想 `tail -f /tmp/server.log` 之后还能在同一条会话里 `ps`、`df -h`、`ip addr`，这样我不用每次重连、不用把几十条固定命令塞进 UI |
| US-4 | 排障中的开发者 | 作为开发者，我想看最近几轮对话的识别耗时与失败原因，这样我能定位"说了话没反应"是 ASR 慢、LLM 慢还是 TTS 没下发 |
| US-5 | 单机自用房主 | 作为房主，我想调唤醒灵敏度和 AEC/波束这些参数，并且**只看到我的设备真的支持的项**，这样我不会对着一个不生效的滑杆调半天 |

---

## 3. 官方六 tab 信息架构逐 tab 拆解

### 3.0 头部、tab 骨架与 tab 集合

来源：`Detail` 组件 `dashboard.jsx:1563` 起；TABS 定义在 `1613-1615`；头部在 `2050-2117`；TabBar `3163`。

| 骨架元素 | 官方行为 | 落位说明（我方） |
|---|---|---|
| tab 集合 | 已批准 + 管理员 → `['status','activity','config','console','updates','logs']`；已批准非管理员 → 去掉 console/updates；**未批准 → `['approve']` 单页** | 我方无用户/角色体系（见 §7.3），一律按管理员视图渲染 |
| tab 默认值 | `device.approved ? 'status' : 'approve'` | 同 |
| 头部左 | 标题（可点击改名，`2056-2065`）、副行 `IP · device_id · firmware_ver · OS·kernel`（`2067-2077`）、`Update available` 琥珀提示 | 复用 `headerRow`；补 P1：点击标题内联改名 |
| 头部右 | LCD 状态牌（`2080-2081`，状态大写字 + 发光）；删除图标 → 行内二次确认（`2083-2092`）；关闭 | 已有状态点/徽标与删除二次确认；补 P1：LCD 状态牌（`insetPanel` + 等宽大写文字） |
| 未批准横幅 | `Action required` + "Name this device below, then approve it to add it to your fleet."（`2113-2116`） | 一期补（现为左菜单"批准"项，需改为单页） |
| tab 切换 | `tabBar`；console tab 独占整个 tab 高度，其他 tab 内部滚动 | 复用 `tabBar` + `splitColumn`；console tab 需占满剩余高度 |

### 3.1 status（状态）

来源：`2168-2390`。结构：两列网格（Panel `Device` ｜ Panel `Resources`）+ 可选 Panel `Bluetooth proxy`。

| 区块 | 控件 / 字段 | 官方数据来源 | 我方数据来源 | 空态 / 错误态 |
|---|---|---|---|---|
| Device | IP（离线带 `(last seen)`） | device.ip / connected | `GET /api/echomuse/devices/{id}` 的 `ip` + `online` | 无 ip → `—` |
| Device | Firmware | device.firmware_ver | `version` | 空 → `—` |
| Device | WiFi network | stats.wifiSsid | `GET …/runtime` → `stats.wifiSsid` | 无上报 → `—` |
| Device | Voice assistant（HA 卫星端口状态） | device.voiceSatellite | **无对应概念**（我方即服务端，无 HA 卫星） | 固定文案：`本服务端（无 Home Assistant 卫星）`，不自造状态 |
| Device | Status（Online / Offline · last seen） | connected + last_seen | `online` + `last_seen` | — |
| Device | Volume（%） | device.volume 或 stats.volumePct | `runtime.volume_state`（设备回报的原始音量） | 未上报 → `—`；**另起一行显示"物理静音：是/否/未知"**（`mute_state`，只读） |
| Device | Link（TLS + token） | `_linkRow(device)` | 我方事实：`ws://` 明文 + `[echomuse].token` 是否校验 + 当前 control 是否 `authenticated` | 三态文案：`明文 ws · token 已校验` / `明文 ws · 未设 token（局域网信任）` / `未认证` |
| Device | Flash wear | device.health.emmc | **未采集** | `—`（不显示等级色） |
| Device | Last boot | health.bootAt + bootReason | `boot_reason` + `boot_id` + `last_seen` 相对时间 | 缺 → `—` |
| Device | Config（Fleet / Local override N/M） | config_sections | **无 fleet/device 作用域概念**（我方只有每设备 settings） | 固定显示 `本机设置（N 项已保存）`，N 来自 settings 键数；不显示 M |
| Device | 配对入口 | pairRequest / pairRequestedAt | `runtime.pair_requested_at` | 有值显示琥珀提示；一期只展示，不做配对动作 |
| Resources | StatBar CPU（`%` + `x/y cores`） | stats.cpuPct/coresOnline/coresTotal | 同名字段全在 `runtime.stats` 白名单内 | 无 stats → 显示 `waiting for device stats…` 等价文案"等待设备上报…" |
| Resources | StatBar RAM（`used / total MB`） | memUsedMb/memTotalMb | 同 | 同上 |
| Resources | StatBar Storage（`x.x / y.y GB`） | storageUsedMb/storageTotalMb | 同 | 同上 |
| Resources | StatTile Link（RSSI dBm + 信号格 + 频段/丢包） | stats.wifiRssi + linkQuality | `wifiRssi`/`wifiFreqMhz` 有；`linkQuality`（每分钟丢包带）**无** | 值 → `—`；丢包带一期不做 |
| Resources | StatTile Latency（ms） | device.rttMs（控制器侧测量） | **未测量** | 恒 `—` + 副标"未测量" |
| Resources | StatTile Temp（°C + 最热区 + 降频） | cpuTempC/maxTempC/thermalCoreLimit | 三个字段全有 | 缺 → `—`；降频判定 `thermalCoreLimit < coresTotal` 可做 |
| Resources | LinkStrip（近 N 分钟丢包条） | linkQuality.minutes | **无分钟级序列** | 一期不做；可选 P2：用 `tcpUpRetrans/tcpUpSegs` 显示**累计**重传率（明确标注"自设备启动累计"，不是近 N 分钟） |
| Bluetooth proxy | Scanner / Adverts seen / Nearby / BT addr / HA 状态 / HCI 错误 | stats.ble + device.bleProxy | 我方有 BLE 广播与事件环（`/ble/adverts`、`/ble/state`） | 一期把"扫描状态/广播条数"并入此区块；HA 转发相关行**不显示**（我方无 HA 转发面，见 `docs/echomuse.md` 蓝牙代理"不做"） |
| 底部 | 系统诊断（固定命令原文输出） | 官方无此区块（官方用 stats 结构化字段） | 我方独有：`GET …/system/info` 原文 | 保留在 status tab 末尾折叠区"系统诊断（原始输出）"，默认收起 |

> 设计约束：stats 缺失必须逐字段显示 `—`，不得因为"其他字段有值"就整块渲染 0。官方 `StatTile` 的 `dim` 语义（`876-907`）就是这个：`value == null` → 显示 `—`。

### 3.2 activity（活动 / 轮次可观测性）

来源：`2394-2413` 装配，主体 `TurnObservability`（`1211` 起，渲染 `1292-1388`）、`WakeSampleReview`（`1127` 起）。数据：`GET /api/devices/{id}/turns`（10s 轮询，仅 tab 打开期间）、`/turns/{id}/audio`、`/wake-samples?limit=50`。

| 区块 | 控件 / 字段 | 官方来源 | 我方现状 | 一期落地 |
|---|---|---|---|---|
| 六个 Lcd 统计位 | State / Turns(last 50) / Success % / Median reply / Near-misses / Underruns | turns[] 聚合 + device.owwNearMisses | **完全没有采集** | 只做前四个：State（`runtime.listen_state` 真实有）、Turns、Success、Median reply（来自新增轮次环形缓冲）；**Near-misses / Underruns 一期显示 `—` + 悬停说明"未采集"** |
| 图例 | Listening / Transcribe / Respond | TURN_STAGES `1110-1114` | 无分阶段时长 | 二期（需要 `vad_end_ms`/`stt_ms`/`tts_url_ms` 三段打点） |
| 每轮堆叠条 | 时间 + 三段色条 + 总时长 + outcome + 播放/下载 | turns[] | 无 | 一期降级：**不分段**，只渲染"时间 · 总时长条（单色）· outcome · 触发来源" |
| 悬停详情 | trigger / 三段耗时 / wake model+score+threshold / noise floor / stt 文本 | turns[] | 无 | 一期：trigger、总耗时、识别文本（截断 90 字）、失败原因；其余显示"—" |
| 唤醒样本复核 | 最近 50 条样本：时间/播放/评分/标注下拉/下载/删除 | `/wake-samples` | 无采集、无存储 | **二期**（需服务端存 WAV + 标注表） |
| 空态 | `No voice turns recorded yet — history starts when the device is next used.` | — | — | 一期文案：`还没有轮次记录——设备下次被使用后开始累计` |

> 诚实标注：activity 是官方六 tab 里我方**缺口最大**的一块。一期只做"能从我方现有会话流程真实打出来的字段"，禁止用估算值填 Near-misses / Underruns / 分阶段时长。

### 3.3 config（配置）

来源：`2416-2503`；顶部 `ConnectivityTab`（`1397-1559`）；中部 scoping 汇总条（`2427-2452`）；主体 `DeviceConfigForm`（`9738` 起，7 个 Stage：`9938`/`10048`/`10196`/`10289`/`10379`/`10439`/`10455`）；底部 `Push config` / `Cancel`（`2490-2501`）。

| 区块 | 控件 / 字段 | 官方来源 | 我方现状 | 落位说明 |
|---|---|---|---|---|
| 顶部：当前连接 | Network / IP / Signal(dBm + 信号格) | `ConnectivityTab` 1477-1488 | 有 `stats.wifiSsid`/`ip`/`wifiRssi` | 迁入 config tab 顶部（左菜单"WiFi 配置"消失） |
| 顶部：可见网络 | Scan/Rescan + 列表（SSID + dBm + `← current`） | `POST /wifi/scan` | 有：`POST …/command {"action":"wifi_scan"}` → `runtime.wifi_scan.networks` | 一期做，含"正在扫描…"/"未发现网络"/扫描失败三态 |
| 顶部：切换网络 | SSID + 口令（显示/隐藏）+ `Switch…` → `Confirm switch`；pending/result 两条结果横幅 | `POST /wifi`，结果经 `device.wifi.{pending,last_result}` | 有：`POST …/command {"action":"wifi"}`、`wifi_result`、`wifi_commit` | 一期做。**文案必须保留官方语义**：切网期间设备会掉线、失败自动回滚、约 2 分钟、`wifi_commit` 只在回滚窗口内有效 |
| 作用域汇总条 | Fleet / Device / Revert all to fleet | `2427-2452` | **无 fleet 概念** | **不显示**（我方只有单机每设备设置）；PRD 明确记录此差异 |
| 7 个 Stage | 见 §6 配置分组映射表 | `CONFIG_SECTIONS` `9578-9586` | 现有 `EchoMuseAudioPanel.kt`/`EchoMuseWakeWord.kt` 散装控件 | 按 §6 重排为 01–07，键不存在的控件不渲染 |
| 底部动作 | `Push config`（脏才显） + `Cancel` | 2490-2501 | 现有"保存" | 一期：保留单按钮保存 + 成功/失败提示；文案改为"下发配置"并注明"202 仅代表已排队" |

### 3.4 console（控制台）

来源：`2506-2510` + `Shell` 组件 `1006-1103`。渲染逻辑极简：

| 区块 | 官方行为 | 落位说明（我方） |
|---|---|---|
| 在线 | 全高 xterm.js + WS `/api/devices/{id}/shell?token=`；tab 打开即挂载、自动 focus | 全高行模式终端（详见 §5） |
| 离线 | 单行琥珀文案 `Device offline — console unavailable` | 同文案语义：`设备离线——控制台不可用` |

### 3.5 updates（更新）

来源：`2513-2889`。官方内容：Firmware 面板（版本/可用版本/回滚槽 + Check now + Update/Rollback + 发布说明折叠）、emOS 面板（更新 + Local payload）、Local Build、Maintenance（debloat）、On-device wake word 资产面板、部署活动控制台。

| 官方区块 | 官方动作 | 我方一期取舍 |
|---|---|---|
| Firmware 面板 + `Check now` + Update/Rollback | GitHub Release 拉取 + A/B 槽推送 | **不做**。理由引既有决策：`docs/echomuse.md`「emOS OTA 仍刻意未实现：需要读取每台设备本机 boot、保留内核/DTB、重建镜像、校验刷写和 trial 回滚，不能套用 ESP OTA，也不能发布通用 boot.img」。一期只保留**只读**：设备上报 `version`、`board`、`base_os`、`kernel_release`
| emOS 面板（含 Local payload 上传 zip） | 推送 emOS 镜像 | **不做**（同上，属固件刷写）。UI 上不出现任何"更新/安装"按钮，出现即违约 |
| Local Build（上传二进制部署） | 推送 `device/build/server` | **不做**（属远程任意文件写，见 §7.3）
| Maintenance（官方是 re-apply debloat） | 幂等重放 payload | **改造为"维护"区**：重启设备（`POST …/system/reboot`，二次确认 + "仅确认请求已发出"）、复制诊断内容（现有）、VAD/ORT 资产重装（`POST …/oww/install-vad`，现有）
| On-device wake word 资产面板 | 状态 installed/outdated/blocked/unknown + 缺失文件数 + 安装进度 + 控制器日志 | **一期保留为只读状态 + 手动重装 VAD**：状态四态文案对齐（`已安装/版本过期/空间不足/设备离线——状态未知/未安装`），进度条与逐行日志列为 P1
| 部署活动控制台 | 本次会话的部署输出（等宽，✓/⚠/Error 着色） | 复用为"资产下发日志"区（与 install-vad 的返回/错误对应）

> 硬约束：updates tab **不得承诺 OTA**。它是一个"版本信息 + 资产状态 + 维护动作"的页面，不是升级入口。PRD 层面直接禁止出现"更新可用/立即更新/回滚"这类文案。

### 3.6 logs（日志）

来源：`2891-2911`。官方行：`时间(11ch 等宽) │ 级别(48px) │ 来源(64px) │ 正文`，一次取最近 50 条，进 tab 拉一次。

| 区块 | 官方 | 我方现状 | 一期落地 |
|---|---|---|---|
| 数据粒度 | 结构化条目 `{ts, level, source, message}`，device 与 controller 混合 | 固定源白名单的**整块文本尾部 48KiB**（`GET …/system/logs?source=`） | 保留源下拉（六源白名单不变），渲染改为"整块等宽输出 + 复制"（现状），**P1** 才做行式解析（解析不出 `ts/level` 的行按正文整行渲染，不丢内容） |
| 空态 | `No logs` / `Loading…` | 现有 | 文案：`无日志` / `读取中…` |
| 错误态 | 官方只 catch console.error，UI 空白 | 有 `{error,code}` | 显示 `{error}` + code 提示（如 `busy/offline/timeout`），不显示空白 |

> 说明：官方把"设备日志"汇总进控制器（`device_logs` 表），我方是"按需读设备文件"。这是一条**架构性差异**，一期不改（不做后台日志汇聚），只把 UI 形状对齐。

### 3.7 未批准设备：审批单页

来源：`2129-2165`。`New Device — Pending Approval` 标题 + 三行（Serial / IP / First seen）+ Label 输入（含说明"会给设备命名，也会是 Home Assistant 里的 `{label} Voice Assistant`"）+ 大按钮 `Approve & Add to Fleet` + 未填标的提示。

我方落位：未批准设备的卡内**不再有左菜单**，直接渲染单页表单；`Home Assistant` 说明改为 `命名会显示在管理台设备列表`（我方无 HA 卫星）。`POST …/approve {"label"}` 已存在。

### 3.8 左菜单七项 → 六 tab 迁移映射（防止功能丢失）

| 现左菜单项 | 迁移后位置 | 备注 |
|---|---|---|
| 概览 | **status** tab | 现有 IP/内核/启动原因行并入 Device 面板 |
| WiFi 配置 | **config** tab 顶部 `ConnectivityTab` 区 | 与官方一致（官方 WiFi 就在 config 顶部） |
| 唤醒词设置 | **config** tab Stage 02；资产状态进 **updates** | 现有强度滑杆/模型下拉/检测位置原样迁入 |
| 音频调优 | **config** tab Stage 01 / 03 / 04 / 05 | VAD/增益/AEC/波束/打断/duck/播报电平按键分派到对应 Stage（见 §6） |
| 音乐（DLNA） | **config** tab Stage 08「音乐（本服务端扩展）」 | 官方无对应物，明确标注非官方；不放进六 tab 之外另开入口，避免破坏对齐 |
| 设备管理（四 tab） | 拆分：状态与音量 → **status**；系统信息 → **status** 底部系统诊断；日志 → **logs**；维护 → **updates** 维护区 | 原"四 tab"外壳整体废弃 |
| 蓝牙代理 | **config** tab Stage 06 的 `after` 区 | 官方同位置挂 blePanel（`10443`） |

---

## 4. 与我方现状的差距表

> "一期/二期"列：P0 = 一期必须；P1 = 一期可选/紧随；P2 = 二期候选；`不做` = 明确排除并记录理由。

| # | 能力 | 官方 | 我方现状 | 落地动作 | 期次 |
|---|---|---|---|---|---|
| 1 | 详情容器 | 六 tab（status/activity/config/console/updates/logs） | 卡内左菜单七项 + "设备管理"内嵌四 tab | 重排为六 tab；左菜单整体移除（映射见 §3.8） | P0 |
| 2 | 设备控制台 | xterm.js + WS，pty 可跑 `vi`/`top` | **完全没有**；只有 `&'static str` 固定命令桥（`shell.rs`） | 新增 WS 会话通道 + 行模式终端 UI（§5） | P0 |
| 3 | 活动可观测性 | 六统计位 + 三段堆叠条 + 录音回放与标注 | **零采集** | 一期只做 6 字段轮次环形缓冲 + 4 统计 + 单色条；录音/分阶段/near-miss 二期 | P0（裁剪）/P2 |
| 4 | 状态：资源条与指标位 | CPU/RAM/存储三进度条 + Link/Latency/Temp 三指标 + 丢包带 + 降频判定 | 只有 `outputBlock` 的整块 runtime JSON 文本 | 结构化渲染；Latency/丢包带/Flash wear 显 `—` | P0 |
| 5 | 配置分组 | 7 个 Stage，每 Stage 带 scope 切换与进阶折叠 | 散装面板（音频调优/唤醒词两个自定义面板），无分组 | 按 §6 重排；无 scope 概念，不显示 Fleet/Device 切换 | P0 |
| 6 | 配置键覆盖 | 设备 `ConfigMessage` 42 键（含 `consolePassword`/`consoleTimeoutMin`/`sendspinUnpaired`） | 白名单 39 键，缺上述 3 键 | 一期只渲染已有 39 键；补 3 键列 P2（需确认设备侧真消费） | P0/P2 |
| 7 | WiFi 配网闭环 | 扫描 → 选网 → 切换（二次确认）→ pending/result 横幅 → 自动回滚 | 有 scan/wifi/commit 与 `wifi_result`，但无 pending 态、无"← current"标记、无二次确认 | 补三态与二次确认；pending 用"已下发，设备正在切换"表达（**不得**写成已切换成功） | P0 |
| 8 | 未批准设备 | 单页审批 + Action-required 横幅 | 左菜单"批准"项 | 改单页审批表单 | P0 |
| 9 | 头部 | 可点击改名 + LCD 状态牌 + 删除二次确认 | 有状态点/徽标与删除二次确认；无改名、无 LCD 牌 | 改名 P1；LCD 状态牌 P1 | P1 |
| 10 | 日志 | 结构化条目（ts/level/source/message），设备+控制器混合，进度式加载 | 六源固定白名单整块文本 | 一期保留整块；行式渲染 P1；后台汇聚不做 | P0/P1 |
| 11 | 固件/emOS OTA、回滚、Local Build、刷写 | 全有（Firmware / emOS / Local Build 面板） | 无，且 `docs/echomuse.md`「刻意不做的范围」明确排除 | **不做**。updates tab 只保留只读版本 + 资产 + 维护 | 不做 |
| 12 | 唤醒样本录音与标注 | `/wake-samples` 列表、播放、WAV 下载、标注、删除 | 无采集无存储 | 二期（需服务端落盘 + 标注表 + 隐私开关） | P2 |
| 13 | 用户与角色 | `isAdmin` 决定 tab 与动作可见性 | 无用户体系；只有管理 token / 设备 token | **不做**（§7.3）；tab 一律管理员视图 | 不做 |
| 14 | 蓝牙代理 | HA 转发面 + BleProxyKey（配对密钥） | 直驱 GATT + 广播采集（无 HA 转发、无配对） | 保留我方 GATT 面板，挂 Stage 06 `after`；HA 相关行不显示 | P0 |
| 15 | 音乐面 | 无（官方不含 DLNA） | 有（扫描/浏览/播放/停止） | 挂 Stage 08 并标注"本服务端扩展" | P0 |
| 16 | 音量/静音 | Volume 行（%）+ 配置项 `startupVolume` 已从 UI 撤掉（官方注释 `10002-10008`） | 有"输出音量（0-127）"输入 + 下发 | status 显示只读音量+静音；下发入口放 status 的"音量"行旁（P1），不放进 config | P0/P1 |

---

## 5. 控制台专章（核心交付）

### 5.1 为什么不能"照抄 xterm.js"

Kuikly 无 DOM、无 `Terminal`/`FitAddon`、无 `ResizeObserver`，官方 `Shell`（`1006-1103`）依赖的库与 API 全部不可用。因此控制台必须落成**行模式终端（line-mode terminal）**：终端仿真退化为"等宽文本缓冲 + 单行输入"。

### 5.2 pty vs 行模式：能力对照与功能等价边界

| 能力 | 官方（`pty:true`） | 我方（行模式） | 等价性结论 |
|---|---|---|---|
| 单命令执行并看输出 | ✅ | ✅ | 等价 |
| 连续会话（`cd`/环境变量/函数在后续命令生效） | ✅（同一 `sh` 进程） | ⚠️ **取决于设备侧**：每次提交如果都新起 `sh -c`，`cd` 不保持 | **必须通过服务端保持同一个 shell 进程**，才能宣称"能跑连续会话"；若固件只能逐条执行，UI 必须明确写"每条命令独立执行，`cd` 不保持" |
| 长输出（`dmesg`、`logcat`） | ✅ 滚动 5000 行 | ✅ 上限 **2000 行 / 200k 字符**，超出丢最旧并提示"输出已截断" | 等价（有明确上限） |
| 交互式提示（`read`、`passwd`、`ssh` 密码） | ✅ | ❌ 输入框只在"整行提交"时发字节，无法对提示逐字符响应 | **不等价，且一期不做**；会话若疑似等待输入（长时间无输出且无提示符），靠空闲超时收尾 |
| 全屏 TUI（`vim`/`top`/`less`） | ✅ | ❌ 光标定位/清屏/重绘无法渲染 | **明确不支持**，进入前文案提示 |
| Tab 补全 / 方向键行内编辑 | ✅（mksh 自己做） | ⚠️ ↑/↓ 做**历史命令**；行内左右移动/补全不支持 | 部分等价；↑/↓ 语义与 mksh 历史一致（对齐官方意图） |
| Ctrl-C | ✅ | ✅（发送 `\x03` 字节） | 等价 |
| resize（cols/rows） | ✅（`0x01` 帧） | ❌ 不做，不发 `0x01` 帧 | 无意义（行模式无光标定位）；但**服务端必须容忍**客户端不发 resize |
| ANSI 颜色/光标控制 | ✅ 渲染 | ⚠️ **必须剥离**（Kuikly 文本不渲染 ANSI），否则用户看到满屏 `\x1b[32m` | 降级：去色 |

### 5.3 帧协议（产品级语义，供工程师核对）

官方语义（`1006-1103` 客户端 + `em_controller.py:5999-6042` 服务端 + `shell_meta`）：

| 方向 | 帧 | 语义 |
|---|---|---|
| 服务端 → 客户端 | TEXT JSON `{"type":"shell_meta","pty":<bool>}` | **任何 shell 字节之前**必须先到一条；`pty` 是设备**实际建立**的模式，不是请求的模式 |
| 服务端 → 客户端 | BINARY / TEXT | 设备 stdout **原样**（含 ANSI 转义、含 `\r`） |
| 客户端 → 服务端 | BINARY `[0x00][stdin bytes…]` | pty 模式的按键原始字节 |
| 客户端 → 服务端 | BINARY `[0x01][cols u16 BE][rows u16 BE]` | pty 模式终端尺寸 |
| 客户端 → 服务端（管道模式） | TEXT `cmd + "\n"` | 无 pty 固件：本地回显 + 行缓冲，`\r` 提交整行 |

我方取值（对齐官方"必须先用 shell_meta 决定输入口径"，但不仿真 pty）：

1. 收到 `shell_meta` 前**不接受输入**，输入框置灰，显示"正在与设备建立会话…"；
2. 无论 `pty` 为 true 还是 false，**我方一律用行模式**：输入整行后发 `[0x00][utf8(cmd + "\n")]`（pty 模式用帧头 0x00，管道模式发 TEXT `cmd+"\n"`——由 `pty` 决定，语义与官方一致，只是不复用 resize/逐键）；
3. 不发送 `0x01` resize 帧；
4. 收到 `pty:false` 时在输出区顶部打一行说明：`设备固件不支持 PTY —— 已降级为行模式`。

### 5.4 输出净化（必须在 UI 层做，进 PRD 硬约束）

| 输入 | 处理 |
|---|---|
| CSI/OSC 等 ANSI 转义序列（`ESC [ …`、`ESC ] … BEL`） | 剥离，保留可见文本 |
| `\r\n` / 单独 `\r`（进度条刷新） | `\r\n` → 换行；单独 `\r` → **覆盖当前行**（`tail -f` 场景可用）；无换行的尾部累积成"当前行" |
| 非 UTF-8 字节（`cat` 二进制） | 每非法字节替换为 `�`，**不得崩溃、不得清空缓冲** |
| `\t` | 展开为 4 空格 |
| 单行超长 | 单行截断 4000 字符并标注"…（行过长已截断）"，避免布局爆炸 |

### 5.5 会话状态机（用户在 UI 上能看到的六种状态）

| 状态 | 触发 | 输入框 | 输出区 | 文案 |
|---|---|---|---|---|
| 连接中 | 点开控制台，WS 建连中 / 等 `shell_meta` | 置灰 | 空 | `正在与设备建立会话…` |
| 已连接 | 收到 `shell_meta` | 可用，自动聚焦 | 可滚动，自动滚到底 | 顶部常驻提示（见 §5.6）；`已连接 · 行模式` |
| 设备离线 | `online=false` | 置灰 | 保留上次缓冲（标注"历史输出"） | `设备离线——控制台不可用`（对齐官方 `2509`）；带"重新连接"按钮 |
| 设备被占用 | 服务端 409 `busy`（另一控制台会话 / 系统信息 / 日志 / 资产下发 / 文件传输） | 置灰 | 保留 | `设备已被另一个操作占用（系统信息/日志/资产下发），请稍后重试` + "重试" |
| 超时断开 | 空闲超时或会话硬上限 | 置灰 | **保留**（不清空） | `会话因 10 分钟无输入已断开`，按钮"重新连接" |
| 鉴权/不支持失败 | 401/403/422（设备 token 错、未批准、非 emOS、控制对端不可用） | 隐藏 | 显示错误 | 按 `{error,code}` 直译：`无权限（invalid device token）`、`设备未批准`、`仅 emOS 设备支持控制台`、`控制对端不可用` |

补充状态：`输出已截断（保留最近 2000 行）`、`连接错误，正在重连…`（3s 起 ×2 封顶 30s，复用 `ConsoleState` 同一退避）。

### 5.6 危险操作提示文案（定稿，不要各写一套）

常驻（终端顶部，等宽小字，弱化色）：

```
此终端以 root 身份在设备上执行命令。命令会影响正在运行的语音服务；重启类命令会立即中断本会话。
不支持全屏交互程序（vim / top / less）。输出上限 2000 行，超出丢弃最早内容。
```

首次进入某设备的控制台时（同一会话只弹一次，inline 说明，非模态）：
```
控制台直接读写设备 root 文件系统。误操作可能让设备无法启动，恢复需要重新刷机。
```

不做命令文本拦截（黑名单不可靠、误报高），改为：**常驻提示 + 全量审计**。

### 5.7 会话生命周期与会话管理（安全底线）

| 项 | 规定 |
|---|---|
| 并发 | **单设备同时只允许一个 shell 会话**；与系统信息/日志/重启/资产下发/文件传输**共用同一把 shell 锁**，双向互斥（任一方占用时另一方 409） |
| 空闲超时 | **建议值 10 分钟**（以"最后一次客户端输入"为计时起点，输出流动不重置）；超时后服务端主动关闭会话，UI 保留输出并显示可重连 |
| 硬上限 | 单会话最长 **2 小时**，到点断开（防忘记关闭长期占用 root 通道） |
| 断开后行为 | 缓冲保留在页面内存（不落盘）；重连是**新会话**，不恢复上一个 shell 的进程状态 |
| 审计 | 每次会话**必须**留痕：时间、device_id、来源 IP、管理凭据是否校验、客户端提交的**每一行命令行**、会话结束原因（用户关闭/空闲超时/硬上限/设备断开/出错）。审计行为 info 档恒定输出，不受 `RUST_LOG=debug` 开关影响 |

---

## 6. 配置 tab 分组映射表

结构对齐官方 `CONFIG_SECTIONS`（`9578-9586`）+ 7 个 Stage（`9938`–`10472`）。**键能否出现，唯一事实来源是 `server/src/device/echomuse/admin.rs::SETTINGS_KEYS`（39 键）**；官方 UI 上有、但设备 `ConfigMessage`（`device/internal/config/config.go`）里没有的键属于**控制器侧能力**，我方不显示（不造无效果控件）。

> 文档纠偏：`docs/echomuse.md` 第 135 行声称白名单含 `ledScene`/`ledListenColor`/`ledThinkColor`/`meter*`/`nsAsr`/`buttonSingleTapEvent`/`buttonMultiTapMs`，与当前代码不符（代码只收 `listeningAnim`）。本 PRD 以代码为准，并在实现时同步修正该文档。

### 6.1 Stage 01 Playback（官方 group `playback`）

| 官方控件 | 键 | 我方白名单 | 控件类型 | 取值 / 单位 / 默认 |
|---|---|---|---|---|
| EQ 六段滑杆 + 曲线 + 预设 | `eqBands` | ✅ `float list` | 一期：预设下拉（Flat / Vocal / Bass）+ JSON 文本域；P1 换滑杆 | 非空数字数组；长度按官方 6 段 |
| — | `eqLoudness` | ✅ `bool` | `switchRow` | 默认 false |
| Speak while the reply is written | `streamReply` | ❌ 控制器侧 | 不显示 | — |
| Response level | `responseLevel` | ✅ `low\|medium\|high` | `dropdownField` | 默认 `low`；选项文案 Low(normal)/Medium(+6dB)/High(+12dB) |
| Duck depth | `duckDb` | ✅ `number -60..0` | `labeledIntField`（P1 滑杆） | 默认 **-18 dB**（官方 UI 默认） |
| Volume button sound | `volumeButtonSound` | ✅ `bool` | `switchRow` | 默认 true（官方 UI 默认） |
| Speaker protection（进阶折叠） | `bassGuardEnabled` | ✅ `bool` | `switchRow` | 默认 true |
| limiter（官方已从 UI 移除，`9975-9993`） | `limiterEnabled`/`limiterThreshold`/`limiterRelease` | ✅ | P2 才出控件（先进阶折叠、默认不发） | `-60..0`、`10..2000` |

### 6.2 Stage 02 Wake word（官方 group `wakeword`）

| 官方控件 | 键 | 白名单 | 控件类型 | 取值 / 单位 / 默认 |
|---|---|---|---|---|
| 模型卡片网格（内置四款 + 自定义 + 上传） | `owwModel` | ✅ `string` | 复用现有 `dropdownField`（官方四款 + 服务端 `oww-models/` 清单 + 已存自定义值） | 空 = 设备默认 `hey_jarvis_v0.1`；"恢复默认"必须显式发该值 |
| Sensitivity（Precise↔Eager） | `owwThreshold` | ✅ `(0,1]` | 复用现有滑杆（**0.1~0.975、步进 0.025、反向**，轨外历史值钉端点） | 服务端校验 `(0,1]` 不变 |
| Speex denoise | `owwSpeexNs` | ❌ 设备无此键 | 不显示 | — |
| Barge-in | `bargeInEnabled` | ✅ `bool` | `switchRow` | — |
| Barge threshold | `bargeInThreshold` | ✅ `number 0..1` | `labeledDecimalField`（P1 滑杆） | 与唤醒阈值同款"Precise/Eager"注释 |
| Arbitration window | `wakeArbitrationMs` | ❌ 控制器侧（多设备仲裁） | 不显示 | — |
| Wake sound | `wakeSound` | ✅ `bool` | `switchRow` | 默认 false |
| Wake sound level | `wakeSoundLevel` | ✅ `quiet\|medium\|loud` | `dropdownField`，**仅 `wakeSound=true` 时显示**（官方 `10142`） | 默认 `medium` |
| Wake word detection | `owwOnDevice` | ✅ `off\|shadow\|on` | `dropdownField`（现有；`shadow` 仅在设备已处于该档时出现） | 默认 `off`（官方推荐 On this Echo，本服务端基线 off） |
| Save wake-word samples / 最小分数 | `wakeClipCapture`/`wakeClipMinScore` | ❌ 未采集 | 不显示（二期） | — |

### 6.3 Stage 03 Microphones（官方 group `microphones`）

| 官方控件 | 键 | 白名单 | 控件类型 | 取值 / 单位 / 默认 |
|---|---|---|---|---|
| 麦克风预设卡片 + DeviceDiagram | （官方组合键） | ❌ 无预设概念 | 一期**不画预设与示意图**（Kuikly 画不了官方那张 7 麦示意），改用下面两个基础控件 | — |
| Beamforming | `beamformingEnabled` | ✅ `bool` | `switchRow` | 设备默认 true（排障首个怀疑对象） |
| Beam angle | `beamAngle` | ✅ `number -1..360` | `labeledIntField` | `-1` = 自动（onset 比选） |
| MICPGA | `adcMicpga` | ✅ `integer 0..127` | `labeledIntField` | 控件量程取官方 **0..59**（校验仍 0..127）；官方默认 40 |
| Digital gain | `adcDigitalGain` | ✅ `integer 0..127` | 同上 | 控件量程 **0..100**；官方默认 88 |
| Mic gain | `micGainDb` | ✅ `integer 0..42` | 同上 | 单位 dB，默认 24 |
| Echo cancel (AEC) | `aecEnabled` | ✅ `bool` | `switchRow` | 设备默认 true |
| AEC delay | `aecDelayMs` | ✅ `integer 0..1000` | 同上 | step 10，单位 ms，默认 250 |
| AEC tail | `aecTailMs` | ✅ `integer 0..500` | 同上 | step 10，单位 ms，默认 300 |
| Echo reference | `aecRefSource` | ✅ `auto\|hw\|sw` | `dropdownField` | 默认 `auto`；`hw` 无硬件环回时禁用 + 原因 |
| Noise suppression | `nsAsr` | ❌ 设备无此键 | 不显示 | — |
| Save utterances | `saveUtterances` | ❌ 未采集 | 不显示（二期） | — |
| Wake word microphone | `wakeMic` | ✅ `integer 0..6` | `dropdownField` | Centre / MK1–MK6；默认 0（中心麦） |

### 6.4 Stage 04 Ring（官方 group `ring`）

| 官方控件 | 键 | 白名单 | 控件类型 | 取值 / 单位 / 默认 |
|---|---|---|---|---|
| 场景卡片（standard/custom 等）+ 颜色选择器 | `ledScene`/`ledListenColor`/`ledThinkColor` | ❌ 设备无此键 | **不显示场景卡与取色器** | — |
| Remote volume arc | `remoteVolumeArc` | ✅ `bool` | `switchRow` | 默认 false |
| 六个 meter 滑杆（Decay/Attack/Gamma/Floor/Reference/Curve） | `meter*` | ❌ 设备无此键 | 不显示 | — |
| （设备侧真实存在的动画说明） | `listeningAnim` | ✅ `json` | P2：`labeledTextArea`（JSON 校验失败即禁止提交） | 对象/数组；默认不发 |

> 该 Stage 一期实际只剩 1 个开关。这是**如实结果**，不要为了"看起来饱满"伪造滑杆。Stage 标题下加一行说明：`设备固件只接受 listeningAnim（JSON），其余灯环参数为官方控制器侧渲染，本服务端不下发。`

### 6.5 Stage 05 Advanced（官方 group `advanced`）

| 官方控件 | 键 | 白名单 | 控件类型 | 取值 / 单位 / 默认 |
|---|---|---|---|---|
| Tap sends an event / Multi-tap window | `buttonSingleTapEvent`/`buttonMultiTapMs` | ❌ 设备无此键 | 不显示 | — |
| Console password / Console idle timeout | `consolePassword`/`consoleTimeoutMin` | ⚠️ 设备有键、我方白名单缺 | **P2** 补白名单后出控件（`labeledTextArea` / `labeledIntField 0..90`） | 0 = 永不；注意这是 **USB 串口 console**，与控制台 WS 会话无关，文案必须区分 |
| Controller address | `controllerEndpoints` | ❌ 我方由 mDNS 决定 | 只读说明：`控制器地址由服务端 mDNS 广播自动下发，无需手工配置` | — |
| Auto gain (AGC) | `agcEnabled` | ✅ `bool` | `switchRow` | 默认 true |
| Threshold（语音门） | `vadThreshold` | ✅ `(0,1]` | `labeledDecimalField` | 控件量程取官方 **0.0001..0.02**；设备默认 0.004 |
| Speech gate | `vadSpeechMs` | ✅ `integer 0..60000` | `labeledIntField` | 量程 **32..320**、step 32、单位 ms，默认 160 |
| Silence gate | `vadSilenceMs` | ✅ `integer 0..60000` | 同上 | 量程 **200..2000**、step 100、单位 ms，默认 800 |
| 现有"JSON 进阶键"手填入口 | 任意白名单键 | ✅ | 保留为最后的分组"进阶（JSON）" | 校验失败即拒绝，错误原因直出 |

### 6.6 Stage 06 Bluetooth（官方 group `bluetooth`）+ after 区

| 官方控件 | 键 | 白名单 | 控件类型 | 取值 / 默认 |
|---|---|---|---|---|
| Bluetooth proxy | `bleProxyEnabled` | ✅ `bool` | `switchRow` | false；`[echomuse].ble_proxy=false` 时禁用 + 原因"服务端未开启蓝牙代理" |
| Allow connections | `bleProxyConnections` | ✅ `bool` | `switchRow`，仅 proxy 开时可用 | false |
| after：BleProxyKey（配对密钥） | — | ❌ 无配对能力 | 不显示 | — |
| after：我方 BLE 面板（广播/槽位/GATT 六种请求） | — | 非配置键 | 保留现有 `EchoMuseBleCard` | 明确"无配对/无加密、`ok:false` 是设备明确拒绝" |

### 6.7 Stage 07 Sendspin（官方 group `sendspin`）

| 官方控件 | 键 | 白名单 | 控件类型 | 取值 / 默认 |
|---|---|---|---|---|
| Sendspin player | `sendspinEnabled` | ✅ `bool` | `switchRow` | false |
| Play without pairing | `sendspinUnpaired` | ⚠️ 设备有、我方缺 | P2 补白名单 | false |
| （我方独有）设备名 | `sendspinName` | ✅ `string` | `labeledField` | 空 = 设备默认 |

### 6.8 Stage 08 音乐（**非官方，本服务端扩展**）

无配置键，纯动作面板：DLNA 扫描 → 选服务器 → 浏览曲目 → 播放所选 / 停止并清空缓冲（现有 `EchoMuseMusicPanel.kt` 内容原样迁入）。Stage 标题标注 `（本服务端扩展，官方 dashboard 无此分组）`。

---

## 7. 非功能要求

### 7.1 Kuikly 没有 DOM —— 控制台只能是行模式终端

- **禁止**引入 xterm.js / 任何 WebView 伪终端渲染；只用 `outputBlock`（等宽、可滚动、自动滚底）+ 现有 `ui/` 原语（`Form.kt`、`Rows.kt`、`Layout.kt`、`Tabs.kt`、`Button.kt`），**不自造控件**。
- 缓冲：上限 **2000 行 / 200k 字符**（复用 `ConsoleState` 的 `CONSOLE_MAX_LINES` 口径），超出丢最旧 + 显式提示"输出已截断"。
- 刷新节流：**200ms 合并刷新**（复用 `ConsoleState.CONSOLE_FLUSH_MS`），避免逐字节重排导致卡顿。
- 交互集（最小必需）：Enter 提交、↑/↓ 历史（本会话内，上限 100 条）、Ctrl-C 中断、清屏、复制全部输出、自动滚底开关。
- 必须提供 §5.2 的**差异对照表**给用户看（放在控制台内的"关于行模式"折叠说明），说明能做什么、不能做什么。

### 7.2 安全与鉴权（控制台毕竟是 root shell）

| 层 | 要求 |
|---|---|
| 管理面鉴权 | 沿用 `auth_ok`：`[server].expected_token` 为空（默认）= 局域网免 token；非空必须 `Authorization: Bearer` 或 `?token=`。WS 无法自定义头，必须支持 `?token=`（浏览器事实约束） |
| 设备侧鉴权 | `[echomuse].token` 校验 `X-Em-Token`；且 shell 回拨须匹配当前已认证 control 的 TCP 来源 IP + 会话探针，**不接受设备自发** |
| 设备准入 | 仅 `approved=true` + `base_os=emos` + 在线 + 当前 control 已认证 + `peer` 可用。其余情况按 §5.5 明确拒绝，不静默超时 |
| 互斥 | 单设备一次一操作（§5.7）；与既有 `shell` 锁共用 |
| 审计 | §5.7 规定；审计内容含**命令行全文**（可能含敏感值，UI 需提示"审计会记录你输入的每一行"） |
| 传输 | 设备链路仍是明文 `ws://`（既有事实）。控制台把这一点放大：连**未设设备 token** 的设备时，控制台顶部加琥珀警告 `设备未设置 token，同网段任何人可连入——root 命令通道不建议在此配置下使用` |

### 7.3 明确不做（范围护栏，实现时不得偷偷加）

| 不做项 | 理由（引用既有决策） |
|---|---|
| emOS OTA / 固件刷写 / recovery / 恢复出厂 | `docs/echomuse.md`「emOS OTA 仍刻意未实现：需要读取每台设备本机 boot、保留内核/DTB、重建镜像、校验刷写和 trial 回滚，不能套用 ESP OTA」「刻意不做的范围：不提供镜像刷写、恢复出厂或 recovery 重启」 |
| 远程任意文件写 / Local Build 部署二进制 | 同上「不提供任意 root 终端、文件上传」——**控制台的 stdin 是命令通道，不是文件上传通道**；`cat > file` 这类用法受输出/输入限长限制，不为其做专门优化 |
| 用户与角色体系（`isAdmin` 等价物） | `docs/echomuse.md` 无用户体系；管理面只有 token 一层。因此六 tab 一律按管理员视角渲染，不实现"非管理员隐藏 console/updates" |
| 后台设备日志汇聚（官方 `device_logs` 表） | 我方是"按需读设备文件"，汇聚属架构变更，不在本次范围 |

**updates tab 的强制约束**：只能出现「版本信息展示」「唤醒词/VAD 资产安装状态」「维护（重启/诊断导出/资产重装）」，**不得出现任何 OTA 承诺**（文案禁止：更新可用、立即更新、回滚到 x.y、安装固件）。

### 7.4 数据诚实性（硬约束）

1. 缺失一律显示 `—` / `未知` / `不可用` + 原因，**禁止填 0**、禁止用上一次会话的值冒充当前值（离线后必须标注"历史"）。
2. `202` / `queued:true` / `dispatched:true` / `delivered:true` 的文案只能是「已下发/已排队/请求已发出」，**不得**表述为「设备已执行/已生效/已重启」。涉及：音量、配置、WiFi 切换、重启、蓝牙 GATT `ok`（写应答 ≠ IoT 动作完成）。
3. `connected_secs` 不是设备 uptime，不得显示为"运行时长"。
4. 物理静音（`mute_state`）与输出音量为 0 是两个状态，不得合并。
5. 活动 tab 的 near-miss / underrun / 分阶段耗时 / 录音，一期显示 `—` 且悬停/副标注明"未采集"，**不估算**。

### 7.5 其它工程约束

| 项 | 要求 |
|---|---|
| 刷新策略 | 不引入全局高频轮询。tab 进入拉一次；activity 打开期间 10s、updates 打开期间 30s，**离开 tab 立即停**（对齐官方 `1617-1669` 的"tab 激活才轮询"） |
| 空态/错误态 | 每个 tab、每个数据块都必须有：加载中 / 空 / 错误 + code / 不可用 + 原因 四种文案之一，禁止空白 |
| 语言 | 简体中文；不出现英文长句（协议 code、字段名、命令原文除外） |
| 无新依赖 | 不新增第三方库；服务端不引入 Pty 库（行模式由设备侧 shell 提供） |
| 回归 | 既有受限 shell（`system/info`、`system/logs`、`reboot`、`install-vad`、文件传输）的行为与 15s 超时、64KiB 上限不许因控制台上线而改变 |

---

## 8. 一期范围与二期候选

### 8.1 一期（P0，必须交付的最小闭环）

| # | 交付 | 验收口径 |
|---|---|---|
| 1 | **六 tab 骨架 + 审批单页** | 设备卡内左菜单消失；已批准设备出现 `状态/活动/配置/控制台/更新/日志` 六 tab（顺序与官方一致）；未批准设备只有单页审批（Serial/IP/First seen + Label + 批准按钮 + Action-required 横幅）；§3.8 七项功能无丢失 |
| 2 | **控制台** | 一次点击进入；`shell_meta` 决定输入口径；`uname -a` 有输出；↑/↓ 历史、Ctrl-C、清屏、复制、自动滚底可用；六种会话状态文案齐全；单设备互斥（与系统信息/日志/资产下发互斥）；空闲 10 分钟断开且输出保留；每次会话产生的审计行可在服务端日志中检索到 |
| 3 | **状态 tab** | Device 面板 8 行 + Resources 三条 + 三个指标位（Latency/Flash wear 恒 `—` 且有副标）+ 音量和物理静音分两行显示；无 stats 时整块"等待设备上报…"；系统诊断折叠区可读原文 |
| 4 | **配置 tab** | 7 个 Stage 全部就位（Playback/Wake word/Microphones/Ring/Advanced/Bluetooth/Sendspin）+ 顶部 WiFi 三块（当前连接/可见网络/切换网络）；只渲染 §6 里标 ✅ 的键；Stage 04 诚实呈现"只有一个开关"+ 说明行；保存按钮文案为「下发配置」且提示"仅代表已排队" |
| 5 | **更新 tab（只读 + 维护）** | 版本信息只读展示（`version/board/base_os/kernel_release`）；VAD/ORT 资产四态状态 + 手动重装；维护区含重启（二次确认 + "仅确认请求已发出"）、复制诊断；**全页无任何 OTA/更新/回滚文案** |
| 6 | **日志 tab** | 六源白名单不变；整块等宽输出 + 复制；四态文案齐全；错误按 `{error,code}` 显示 |

### 8.2 二期候选（P1/P2，按价值排序）

| 期次 | 项 | 依赖 |
|---|---|---|
| P1 | 日志行式渲染（ts/level/source/message），解析失败回退整块 | 无（纯前端） |
| P1 | 头部点击改名 + LCD 状态牌；status 音量下发入口 | 无 |
| P1 | 资产下发进度条 + 逐行日志（官方 `2811-2852` 形状） | 服务端需暴露进度事件 |
| P1 | 配置滑杆化（EQ 六段、灵敏度已具备、其余数字项换滑杆）+ 进阶折叠分组 | 无 |
| P2 | **活动 tab 完整版**：分阶段堆叠条（Listening/Transcribe/Respond）、near-miss、underrun、悬停详情里的 wake score/noise floor | 服务端三段打点 + 影子评分交叉数据 |
| P2 | **录音回放与标注**：utterance WAV 回放/下载、唤醒样本复核列表（播放/标注/删除） | 服务端落盘 + 保留策略 + 隐私开关（默认关） |
| P2 | 配置键补齐：`consolePassword`/`consoleTimeoutMin`/`sendspinUnpaired`；`listeningAnim` JSON 编辑 | 需在真机确认设备侧消费 |
| P2 | 蓝牙代理 status 区块（扫描状态/广播条数/槽位）并入 status tab | 无 |
| P2 | 累计重传率（`tcpUpRetrans/tcpUpSegs`，明确标注"自设备启动累计"） | 与官方"近 N 分钟"不同口径，需文案把关 |

---

## 9. 待确认问题（只列真正影响实现的）

| # | 问题 | 影响 | 建议默认值（若不回复则按此实现） |
|---|---|---|---|
| Q1 | 目标固件（v2.18.1 / `2f3523088af1`）的 shell 会话是**单进程持续**（`cd` 生效、环境变量保持）还是**逐条 `sh -c`**？ | 决定控制台能否宣称"连续会话"；决定 §5.5 是否要加"每条命令独立执行"提示 | 先按"逐条执行"实现并明示；真机确认后再放开 |
| Q2 | 控制台空闲超时取 10 分钟是否可接受？超时后是否需要保留输出（建议保留、可一键重连）？是否要 2 小时硬上限？ | §5.7 生命周期 | 10 分钟空闲 / 保留输出 / 2 小时硬上限 |
| Q3 | 审计日志落哪里：仅服务端日志（`RUST_LOG info` 恒定输出）就够，还是要单独落文件（如 `/data/echomuse_audit.log`，可导出）？ | 审计可追溯性与隐私口径 | 一期只打服务端日志（含命令行全文），单独文件列 P2 |
| Q4 | 活动 tab 一期是否接受"服务端新增内存轮次环形缓冲（50 条、不落盘、重启清空）"？若不接受，activity 一期退化为空态说明 | activity tab 一期能否有内容 | 接受；缓冲区随设备会话生命周期，不落盘 |
| Q5 | 官方有、我方无的键（`consolePassword`/`consoleTimeoutMin`/`sendspinUnpaired`）在配置页要"禁用态 + 原因展示"还是"完全不显示"？ | 配置页完整度 vs 诚实度 | **完全不显示**（不显示未实现的能力，避免"有控件但不生效"） |

---

## 附：核对索引（工程师对照用）

| 内容 | 位置 |
|---|---|
| 六 tab 集合 / 默认 tab | `dashboard.jsx:1613-1615` |
| 头部（改名 / LCD / 删除） | `dashboard.jsx:2050-2117` |
| 未批准审批页 | `dashboard.jsx:2129-2165` |
| status tab | `dashboard.jsx:2168-2390` |
| activity tab | `dashboard.jsx:2394-2413`；`TurnObservability` 1211-1388；`WakeSampleReview` 1127-1209 |
| config tab（含 WiFi 顶部） | `dashboard.jsx:2416-2503`；`ConnectivityTab` 1397-1559；`DeviceConfigForm` 9738-10475 |
| console tab + Shell | `dashboard.jsx:2506-2510`；`Shell` 1006-1103 |
| updates tab | `dashboard.jsx:2513-2889`（维护区 2711-2745；唤醒资产 2748-2868） |
| logs tab | `dashboard.jsx:2891-2911` |
| `CONFIG_SECTIONS` / `SECTION_LABELS` / `STATE_KEYS` | `dashboard.jsx:9578-9600` |
| `shell_meta` 下发与代理 | `controller_em_controller.py:5999-6042` |
| 设备 shell 会话（pty/管道、`ShellActive`） | `device_cmd_server.go:355-381`、`:793-799`、`:872` |
| 设备配置键全量 | `device_internal_config_config.go`（42 键） |
| 我方白名单（唯一事实来源） | `server/src/device/echomuse/admin.rs:161-213` |
| 我方固定命令 / 日志源 | `server/src/device/echomuse/system.rs:17-37` |
| 我方 runtime 字段 | `server/src/device/echomuse/runtime.rs:67-92`、`:169-186` |
| 我方现状说明与"刻意不做的范围" | `docs/echomuse.md` §管理面 API / §emOS Web 管理（安全子集）/ §刻意不做的范围 / §验证边界 |
| 可复用的控制台节流与退避 | `client/shared/.../ConsoleState.kt:238-247` |
