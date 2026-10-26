# EchoMuse 设备管理界面对齐官方 + Web 控制台 · QA 验收报告

> 作者：严过关（QA）｜日期：2026-10-26｜基线：`main@da6f55a`（工作区未提交）
> 验收基准：`docs/plans/echomuse-admin-parity-prd.md`、`docs/plans/echomuse-admin-parity-design.md`
> 原则：**所有结论以本人独立跑出的结果为准**，不采信实现者自报。
>
> ⚠️ **工作区状态提示（给主理人）**：我开工时文件是未暂存（` M` / `??`），结束时 `git status` 显示**已被 `git add` 进索引**（`A ` / `M `）——**不是我做的**（我全程只跑只读 git 命令，未 add/commit）。当前有两个文件仍是未跟踪状态，若准备提交请一并纳入或排除：
> `docs/plans/echomuse-admin-parity-qa-report.md`（本报告）、`server/src/device/echomuse/console_path_tests.rs`（我补的 3 条测试）；
> 另外 `server/src/device/echomuse/console.rs` 是 `AM`（暂存后又改过：我在末尾加了 `path_tests` 的 3 行模块声明）。

---

## 0. 一句话结论

**可以交付（建议带 4 条 P2 一起合入或紧接着修）**：核心主链「点设备 → 控制台 → 输命令 → 看到输出 → 断开会话释放槽」**在本机真实端到端跑通**（含真实 axum 服务端 + 真实 `tools/echomuse_mock_device.py` 假设备 + 真实 WS 客户端），后端 890 通过 / 1 既存失败，前端四端编译全绿、72 条 JVM 单测全绿。**未发现 P0/P1 缺陷**；4 条 P2 均为「文案/文档/测试覆盖/文件行数」，不阻塞功能。

---

## 1. 六项验证的原始结果

### 1.1 后端测试（独立跑）

```
$ ~/.cargo/bin/cargo test --manifest-path server/Cargo.toml
test result: FAILED. 890 passed; 1 failed; 6 ignored; 0 measured; 0 filtered out; finished in 3.63s
```

唯一失败：

```
---- device::echomuse::dlna::tests::duration_formats stdout ----
thread '...' panicked at src/device/echomuse/dlna_tests.rs:34:5:
assertion `left == right` failed
  left: 75780
 right: 201
```

**已独立确认这是「改动前就失败」的既存 bug**，不计入本次回归：

```
$ git diff HEAD -- server/src/device/echomuse/dlna.rs server/src/device/echomuse/dlna_tests.rs
（空输出）
$ git log --oneline -2 -- server/src/device/echomuse/dlna.rs server/src/device/echomuse/dlna_tests.rs
da6f55a feat(session,echomuse): 入口即定型设备类型 DeviceKind…   ← 本轮之前
45ee965 feat(echomuse): 设备 VAD 生效后服务端跳过检测直接 ASR…
```

根因（我逐行确认，不是转述）：`dlna.rs:300-303` 用 `parts.iter().enumerate().rev()` 反向遍历，但乘的仍是**正向**下标 `60^i`：

```rust
for (i, p) in parts.iter().enumerate().rev() {
    let val: f64 = p.parse().unwrap_or(0.0);
    secs += val * 60f64.powi(i as i32);   // i 是正向下标，应为 len-1-i
}
```

`"0:03:21.000"` → 21×60² + 3×60¹ + 0 = **75780**（正是失败输出），正确值 201。**归属既有技术债，非本轮引入**（本轮未触碰 `dlna.rs`/`dlna_tests.rs`）。

已知 flake `workflow::session::producer::tests::all_turn_logs_carry_the_same_turn_id`：本轮两次全量运行**均未出现**，无需重跑。

控制台模块专项：

```
$ ~/.cargo/bin/cargo test --manifest-path server/Cargo.toml console::
running 14 tests
... 14 passed; 0 failed
```

（11 条来自工程师的 `console_tests.rs`，3 条来自本报告 §4.1 我补的 `console_path_tests.rs`）

```
$ DOCS_RS=1 ~/.cargo/bin/cargo check --manifest-path server/Cargo.toml --features sherpa
warning: method `is_noop` is never used  --> src/config/defaults.rs:199:12
warning: `xiaozhi-server-rust` (bin "xiaozhi-server-rust") generated 1 warning
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.27s
```

唯一 warning 在 `config/defaults.rs`（本轮未触碰的模块，既存）。

```
$ ~/.cargo/bin/cargo fmt --manifest-path server/Cargo.toml --check
（无输出）
fmt_rc=0
```

### 1.2 前端四端编译 + 单测（独立跑，全部带 `--rerun-tasks`）

| 目标 | 结果 | 命令尾部 |
|---|---|---|
| `:shared:compileKotlinJs` | ✅ | `BUILD SUCCESSFUL in 3s / 2 actionable tasks: 2 executed` |
| `:shared:compileDebugKotlinAndroid` | ✅ | `BUILD SUCCESSFUL in 4s / 7 actionable tasks: 7 executed` |
| `:shared:compileKotlinMacosArm64` | ✅ | `BUILD SUCCESSFUL in 4s / 2 actionable tasks: 2 executed` |
| `:adminCoreTests:jvmTest` | ✅ | `BUILD SUCCESSFUL in 2s / 4 actionable tasks: 4 executed` |

四端只剩既有的 `expect/actual` Beta 提示与 `ConsoleState.kt` 的 `observable` 弃用告警（均为本轮之前就有的告警，且不在本轮新增文件里）。

单测汇总（`client/adminCoreTests/build/test-results/jvmTest/*.xml`，时间戳为本轮运行时刻，确认真跑）：

| 测试类 | tests | failures | errors | skipped |
|---|---|---|---|---|
| `EchoMuseConsoleTextTest`（本轮新增，含我补的 2 条） | 17 | 0 | 0 | 0 |
| `EchoMuseWakeWordTest` | 15 | 0 | 0 | 0 |
| `FilesModelTest` | 15 | 0 | 0 | 0 |
| `PipelineWorkflowTest` | 8 | 0 | 0 | 0 |
| `UiContractsTest` | 8 | 0 | 0 | 0 |
| `NodeTestModelsTest` | 5 | 0 | 0 | 0 |
| `PipelineFlowTest` | 4 | 0 | 0 | 0 |
| **合计** | **72** | **0** | **0** | **0** |

### 1.3 前后端字面量对账（逐字核）

**帧 type（server → browser）** —— 服务端由 `#[serde(tag="type", rename_all="kebab-case")]` 派生：

| 项 | 后端值 | 前端值（`ConsoleWire`） | 一致 |
|---|---|---|---|
| Ready | `ready` | `READY = "ready"` | ✅ |
| Output | `output` | `OUTPUT = "output"` | ✅ |
| Notice | `notice` | `NOTICE = "notice"` | ✅ |
| Error | `error` | `ERROR = "error"` | ✅ |
| Closed | `closed` | `CLOSED = "closed"` | ✅ |
| Pong | `pong` | `PONG = "pong"` | ✅ |

**帧 type（browser → server）**：

| 项 | 后端 `parse_incoming` | 前端值 | 一致 |
|---|---|---|---|
| input | `"input"` | `INPUT = "input"` | ✅ |
| ctrl-c | `"ctrl-c"` | `CTRL_C = "ctrl-c"` | ✅ |
| ping | `"ping"` | `PING = "ping"` | ✅ |
| clear | 不存在（`Unknown`） | 不存在 | ✅ |

**`error.code`（5 个，缺一不可多一不可）**：

| 后端 `ErrorCode` | 值 | 前端 | 一致 |
|---|---|---|---|
| Busy | `busy` | `CODE_BUSY` | ✅ |
| InputTooLong | `input-too-long` | `CODE_INPUT_TOO_LONG` | ✅ |
| RateLimited | `rate-limited` | `CODE_RATE_LIMITED` | ✅ |
| DeviceOffline | `device-offline` | `CODE_DEVICE_OFFLINE` | ✅ |
| Internal | `internal` | `CODE_INTERNAL` | ✅ |

**`closed.reason`**：

| 后端 `CloseReason` | 值 | 前端 | reason→文案 | 一致 |
|---|---|---|---|---|
| IdleTimeout | `idle-timeout` | `REASON_IDLE` | `IDLE_TIMEOUT`「…10 分钟无输入…」 | ✅ |
| SessionLimit | `session-limit` | `REASON_LIMIT` | `SESSION_LIMIT`「…2 小时上限…」 | ✅ |
| DeviceOffline | `device-offline` | `REASON_OFFLINE` | `DEVICE_LOST` | ✅ |
| Superseded | `superseded` | `REASON_SUPERSEDED` | `SUPERSEDED` | ✅ |
| DeviceShellLost | `device-shell-lost` | `REASON_SHELL_LOST` | `SHELL_LOST` | ✅ |
| Replaced | `replaced` | `REASON_REPLACED` | **无分支 → 落 `else` = `RECONNECT_LOST`** | ⚠️ 见 D1 |
| ServerShutdown（保留码） | `server-shutdown` | `REASON_SERVER_SHUTDOWN` | `RECONNECT_LOST` | ✅ |
| Internal | `internal` | `REASON_INTERNAL` | **无分支 → 落 `else` = `RECONNECT_LOST`** | ⚠️ 见 D1（**实测会发生**） |
| ClientClosed（仅审计） | `client-closed` | 不需要（不发帧） | — | ✅ |

**其它共享字面量**：

| 项 | 后端 | 前端 | 一致 |
|---|---|---|---|
| `notice.code` | `NoticeCode::OutputCapped` → `output-capped` | **无字面量常量**（`NOTICE` 分支只读 `text`） | ⚠️ 单向锚点（见 D5，非缺陷） |
| `ready.mode` | `MODE_PIPE = "pipe"` | `EM_CONSOLE_MODE_PIPE = "pipe"` | ✅ |
| `ready.insecure` | `bool` | `optBoolean("insecure")` | ✅ |
| `ready.notice` | `COPY_READY_NOTICE` | 直接显示服务端文本 | ✅ |

**服务端定值文案 vs 前端 `ConsoleCopy` 逐字比对**：

| 后端常量 | 前端常量 | 一致 |
|---|---|---|
| `COPY_BUSY`「设备已被另一个操作占用（系统信息/日志/资产下发），请稍后重试」 | `BUSY` 同文 | ✅ |
| `COPY_INPUT_TOO_LONG`「单行命令过长（上限 4096 字节），未发送」 | `INPUT_TOO_LONG` 同文 | ✅ |
| `COPY_RATE_LIMITED`「输入过快，已丢弃（每次最多 10 条/秒）」 | `RATE_LIMITED` 同文 | ✅ |
| `COPY_DEVICE_OFFLINE`「设备离线——控制台不可用」 | `OFFLINE` 同文 | ✅ |
| `COPY_INTERNAL`「控制台内部错误：」+detail | 无独立常量（直接用服务端 `error.error`） | ✅ 语义等价 |
| `COPY_CAPPED`「会话输出已达 16 MiB 上限…」 | `CAPPED` 同文 | ✅ |
| `COPY_READY_NOTICE`（行模式说明） | 由 `ready.notice` 下发 | ✅ |

**常量数值（§8.2 两侧同值）**：`DIAL_TIMEOUT 15s`／`IDLE_TIMEOUT 600s`／`SESSION_MAX 7200s`／`SESSION_BYTES_CAP 16 MiB`／`LINE_MAX 4096`／`INPUT_PER_SEC 10` 与前端 `EM_CONSOLE_LINE_MAX 4096`／`EM_CONSOLE_FLUSH_MS 200`／`EM_CONSOLE_HISTORY_MAX 100`／`EchoMuseConsoleText.MAX_LINES 2000`／`MAX_CHARS 200_000`／`LINE_CHAR_MAX 4000` **全部一致**。

**结论**：五个字面量集合（type / error.code / closed.reason / notice.code / ready.mode）**无一处不一致**；唯一偏差在两处 reason 的**文案映射**（D1）。

### 1.4 边界与并发

| 验证点 | 自动化覆盖 | 我的独立验证 | 结论 |
|---|---|---|---|
| 槽互斥 + 持有者 owner 不变 + `Busy` 方不发 `shell_close` | ✅ `busy_contest_never_touches_holder_slot_or_sends_close` | 复跑通过 | ✅ |
| `Drop` 顺序：`shell_close` 先入队、槽后释放 | ✅ `teardown_enqueues_close_before_new_owner_opens` | 复跑通过 | ✅ |
| 回拨超期/二次回拨 → `Unsolicited` | ✅ `stale_and_duplicate_dial_are_unsolicited` | 复跑通过 | ✅ |
| **控制台占槽时 `/system/info` → 409 busy** | ✅ `console_slot_keeps_one_shot_paths_and_whitelist_intact` | 端到端实测 409 `{"code":"busy"}` | ✅ |
| **反向：一次性占槽 → 控制台 `error{busy}`** | ❌ 原无 → **我补** `console_gets_busy_frame_when_one_shot_holds_slot` | 端到端实测首帧 `error{busy}` | ✅（补测后覆盖） |
| **浏览器在 dial 阶段关闭 → 槽秒级释放** | ❌ 原无 → **我补** `browser_close_during_dial_releases_slot_without_waiting_for_dial_timeout` | 端到端实测：关掉后第二个控制台立即进 dial（未拿 busy），`elapsed < 3s` | ✅（补测后覆盖） |
| 设备先断（`device-shell-lost`） | ❌ 无自动化 | 端到端实测收到 `closed{reason:"device-shell-lost",inputs:2,bytes_out:0,secs:1}` | ⚠️ 行为正确、无自动化（D3） |
| `device-offline` / `superseded` 收尾 | ❌ 无自动化 | 未构造（需断 control 通道/重拨替换 hub 条目） | ⚠️ 未覆盖（D3） |
| 空闲 600 s → `idle-timeout` | ❌ 无自动化 | **端到端实测 600.0 s 收到 `closed{reason:"idle-timeout",secs:600}`**（§1.6） | ✅ 行为正确、无自动化（D3） |
| 会话上限 7200 s → `session-limit` | ❌ 无自动化 | **未验证**（需 2 小时真实等待，见 §5） | ⚠️ 未验证 |
| UTF-8 切在 4 KiB 边界、无乱码、字节不丢 | ✅ `utf8_carry_joins_char_split_at_frame_boundary`（4094×`a` + 「一」+ `z`，按 4096 分块） | 复跑通过 | ✅ |
| 非法 UTF-8 → `�` 且会话不断 | ✅ `utf8_carry_replaces_invalid_bytes_and_keeps_going` | 复跑通过 | ✅ |
| 第 11 帧 `rate-limited` 且拒绝不消耗令牌 | ✅ `rate_limit_and_long_line_write_nothing` | 端到端实测 12 连发收到 `rate-limited` | ✅ |
| 4097 字节 `input-too-long` 且**设备侧零字节** | ✅ 同函数（`Reject` 不携带字节，类型上成立） | 端到端实测收到 `input-too-long` | ✅ |
| 16 MiB 后只发一次 `output-capped`、会话不断 | ✅ `budget_caps_once_without_ending_session`（`Forward→Cap→Drop`） | 端到端未跑（需灌 16 MiB；纯函数已钉住） | ✅ 单测 |
| 前端 ANSI 跨块 ESC（`"\u001b[3"` + `"1m…"`） | ✅ `ansi_csi_is_stripped_across_chunk_boundary` | 复跑通过 | ✅ |
| `\r` 覆盖当前行 | ✅ `carriage_return_overwrites_current_line` + `carriage_return_keeps_untouched_tail` | 复跑通过 | ✅ |
| 2000 行 / 200 000 字符裁剪、单行 4000 截断 | ✅ 三条独立用例 | 复跑通过 | ✅ |
| `\t` → 4 空格、OSC 剥离、非法字节 → `�` | ✅ | 复跑通过 | ✅ |

### 1.5 端到端可行性 —— **做成**

实现者报告「本机起不来服务端（缺 `/data/models/SenseVoiceSmall/tokens.txt`）」。我**独立复现了这一点**：

```
$ XIAOZHI_CONFIG_DB=/tmp/qa/e2e/data.db ./server/target/debug/xiaozhi-server-rust
… sherpa-onnx/csrc/offline-model-config.cc:Validate:105 tokens: '/data/models/SenseVoiceSmall/tokens.txt' does not exist
Error: 构建 asr.sensevoice 失败：创建 SenseVoice 识别器失败（检查模型路径或原生库）
```

`/data` 在本机不可创建（`mkdir -p /data` → `ENOENT`），且 `ASR_ENGINES` 只有 `sensevoice` 一条真实路径（`plugins/asr/mod.rs:21`），`Engines::new` 是**硬失败**（`runtime/mod.rs:115` 注释「无 mock：缺模型/缺 feature 直接报错」）。**但存在可行路径，我走通了**：

1. 下载三个模型文件到 `/tmp/qa/models`（共 ~240 MB，`huggingface.co` 3 MB/s）：
   - `SenseVoiceSmall/model.int8.onnx`（239 MB）+ `tokens.txt`（316 KB）
   - `silero_vad.onnx`（644 KB，GitHub release）
2. 预置配置库 `cfg_documents.doc`（SQLite 一行 JSON），只改路径 + 把 `[tts].engine` 切到 `xfyun`（避开 Kokoro 模型）：
   `asr.model/tokens → /tmp/qa/models/…`、`vad.model → /tmp/qa/models/silero_vad.onnx`、`memory.db_path → /tmp/qa/e2e/memory.db`、`echomuse.music_dir → /tmp/qa/music`
3. `XIAOZHI_MODELS_DIR=/tmp/qa/models`（OWW 语音门/ORT 走这里，避免只读 `/data` 告警）
4. 启动 → `GET /api/health` = 200，`[echomuse] 控制器专用端口已监听 0.0.0.0:8767`
5. `python3 tools/echomuse_mock_device.py --port 8000 --device-id mock-biscuit-001 --stay 900`（首轮未批准会收到 `{"type":"pending"}` 并退出 → `POST …/approve` → 重启 mock）
6. `GET /api/echomuse/devices/mock-biscuit-001` → `online: true, approved: true, base_os: emos`（TTS 因 dummy 凭据不出音，mock 打印「⚠️ 回复为空」后照常进入 `--stay`，不影响控制台）

**实测结果（真实 WS 打 `/api/echomuse/devices/mock-biscuit-001/console`）**：

```
① 首帧: {"type":"ready","device_id":"mock-biscuit-001","mode":"pipe","insecure":true,
         "notice":"行模式：同一 sh 会话，cd/环境变量保持；本终端不回显、无提示符。审计会记录你输入的每一行命令"}
② ping → {"type":"pong"}
③ uname -a → {"type":"output","text":"Linux emos 4.9.0-emos #1 SMP PREEMPT Wed Nov 6 10:12:04 UTC 2019 armv7l GNU/Linux\n"}
④ 4097 B → {"type":"error","code":"input-too-long","error":"单行命令过长（上限 4096 字节），未发送"}
⑤ 12 连发 → {"type":"error","code":"rate-limited","error":"输入过快，已丢弃（每次最多 10 条/秒）"}
⑥ ctrl-c 后 ping → {"type":"pong"}（会话存活）
```

**互斥与收尾**：

```
A) 控制台占槽 → GET /system/info = 409 {"code":"busy","error":"another system operation is active"}
B) 一次性操作占槽 → 控制台首帧 {"type":"error","code":"busy","error":"设备已被另一个操作占用（系统信息/日志/资产下发），请稍后重试"}
C) 输入 exit（假 shell 关回拨通道）→ {"type":"closed","reason":"device-shell-lost","detail":"设备关闭了 shell 通道（sh 退出或重启）","inputs":2,"bytes_out":0,"secs":1}
D) 设备不回拨（--no-shell）→ 浏览器关掉后槽立即释放（第二个控制台立即进 dial，未拿 busy）
E) 设备不回拨（--no-shell）且不关浏览器 → 15.0 s 后
   {"type":"closed","reason":"internal","detail":"设备未在 15 秒内回拨 shell 通道","inputs":0,"bytes_out":0,"secs":15}
```

**审计行**（服务端日志，三类齐全、含命令全文）：

```
[echomuse] 控制台会话开启 device=mock-biscuit-001 peer=127.0.0.1 auth=true insecure=true
[echomuse] 控制台命令   device=mock-biscuit-001 peer=127.0.0.1 cmd=uname -a
[echomuse] 控制台命令   device=mock-biscuit-001 peer=127.0.0.1 cmd=^C
[echomuse] 控制台会话结束 device=mock-biscuit-001 peer=127.0.0.1 reason=client-closed secs=3 inputs=9 bytes_out=242
[echomuse] 控制台会话结束 device=mock-biscuit-001 peer=127.0.0.1 reason=device-shell-lost secs=1 inputs=2 bytes_out=0
```

**顺带验证的 T02 两个端点（真实服务端）**：

```
GET /api/echomuse/devices/mock-biscuit-001/runtime
→ {"online":true,"link":{"authenticated":true,"token_required":false},"volume_state":null,"mute_state":null}

GET /api/echomuse/devices/mock-biscuit-001/oww/state   (200)
→ {"device_id":"mock-biscuit-001","free_mb":null,
   "states":["installed","outdated","blocked","unknown","missing"],
   "vad":{"state":"unknown","file":"silero_vad.onnx","sha256":"da5e…","bytes":1806511},
   "ort":{"state":"unknown","file":"libonnxruntime.so","sha256":"1742…"},"checked_at":1793025926}
```

`state=unknown`（而不是 `missing`）与设计 §3.2「读不到 ≠ 缺失」一致；`free_mb=null` 也不假填 0。

**可复现的一键步骤**（有模型的环境可跳过第 1–3 步）：

```bash
# 1) 模型（约 240MB）
M=/tmp/qa/models; mkdir -p $M/SenseVoiceSmall
R=https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main
curl -L -o $M/SenseVoiceSmall/model.int8.onnx $R/model.int8.onnx
curl -L -o $M/SenseVoiceSmall/tokens.txt     $R/tokens.txt
curl -L -o $M/silero_vad.onnx \
  https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx
# 2) 空库起一次让它自举（会因缺模型退出，但 data.db 已落默认文档）
XIAOZHI_CONFIG_DB=/tmp/qa/e2e/data.db ./server/target/debug/xiaozhi-server-rust
# 3) 改 data.db 的 asr.model / asr.tokens / vad.model（+ tts.engine=xfyun 若不想装 Kokoro）
# 4) 起服务端
XIAOZHI_CONFIG_DB=/tmp/qa/e2e/data.db XIAOZHI_MODELS_DIR=$M \
  ./server/target/debug/xiaozhi-server-rust
# 5) 假设备：先注册 → 批准 → 再带 --stay 起
python3 tools/echomuse_mock_device.py --port 8000 --device-id mock-biscuit-001 --stay 5
curl -X POST localhost:8000/api/echomuse/devices/mock-biscuit-001/approve \
     -H 'Content-Type: application/json' -d '{"label":"QA"}'
python3 tools/echomuse_mock_device.py --port 8000 --device-id mock-biscuit-001 --stay 900
# 6) 控制台：python3 /tmp/qa/console_probe.py（raw WS 探针，复用 tools 里的 WsClient）
```

### 1.6 空闲超时（600 s）真实等待 —— **做成**

开一条控制台会话后**完全静默**（不发任何帧，包括不发应用层 ping），真实等待 600 秒：

```
[600.0s] 帧: {"type":"closed","reason":"idle-timeout","detail":"会话因 600 秒无输入断开",
              "inputs":0,"bytes_out":0,"secs":600}
IDLE_OK elapsed=600.0s reason=idle-timeout
```

服务端审计同步落一行（22:44:15 开启 → 22:54:15 结束）：

```
[echomuse] 控制台会话结束 device=mock-biscuit-001 peer=127.0.0.1 reason=idle-timeout secs=600 inputs=0 bytes_out=0
```

要点：**输出流动不重置计时**（本会话全程无输出，所以这条只证明"无输入即超时"）、`secs=600` 精确、`inputs=0`、断开后输出保留（前端 `closed` 分支不清 buffer）。

### 1.7 回归与不破坏性

| 检查 | 命令 | 结果 |
|---|---|---|
| 左菜单彻底消失 | `grep -rn "ECHOMUSE_MENU_" client/shared/src/` | **零命中** ✅ |
| 左菜单状态机残留 | `grep -rn "selectMenu\|activeMenuItem\|menuItem\b\|menuDevice"` | **零命中** ✅ |
| 六 tab 名字与顺序 | `EchoMuseDetailState.kt:521-531` | `状态/活动/配置/控制台/更新/日志`，与官方 §3.0 顺序一致 ✅ |
| 更新 tab 零 OTA 文案 | `grep -n "OTA\|刷写\|recovery\|恢复出厂\|更新可用\|立即更新\|回滚" EchoMuseUpdatesTab.kt` | **零命中**（rc=1） ✅ |
| 不把 202 写成「已生效」 | `grep -rn "已生效\|已执行\|已重启" client/.../EchoMuse*.kt` | **零命中**（rc=1） ✅ |
| 白名单恰好 40 键、含 `sendspinUnpaired`、不含 `vadChannel`/`ledScene`/`ledListenColor`/`ledThinkColor`/`meter*`/`consolePassword`/`consoleTimeoutMin` | 脚本解析 `SETTINGS_KEYS` + `admin_tests::settings_keys_are_exactly_the_device_wire_contract` | 40 键，逐项符合 ✅（另 `listeningAnim` 在册） |
| `shell.rs` 改动仅可见性 | `git diff HEAD -- shell.rs` | 全是 `pub(super)` + 注释，**逻辑零变化** ✅ |
| `/system/*` 固定命令与日志六源未变 | `console_tests::console_slot_keeps_one_shot_paths_and_whitelist_intact` | `INFO.starts_with("uname -a && cat /etc/os-release")`、`REBOOT.contains("kill -TERM 1")`、六源全在、`log_command("ledScene").is_none()` ✅ |
| `system.rs` 仅加 `link` | `git diff HEAD --stat -- system.rs` | `4 +++-`（`payload_with_link` + `token_required`），路由与其它逻辑未动 ✅ |
| `serve_with_keepalive`、`admin_ws` 版本事件 | `git status` | 未出现在改动清单（`serve.rs` / `admin ws` 文件零改动） ✅ |
| `labeledField` 新增 `enabled`/`onSubmit` 向后兼容 | `git diff -- ui/Form.kt` | 二者均有默认值（`{ true }` / `null`），旧调用方零改动 ✅ |
| 单文件 ≤600 行 | `wc -l` | 见下 |

**本轮新增/改动的 `.rs`（全部 ≤600）**：

```
531→537 console.rs        （+6：path_tests 模块声明 + 注释）
375     console_frames.rs
584     console_tests.rs  （工程师原样；我的补测另起文件）
233     console_path_tests.rs（QA 新增）
168     oww_state.rs
122     oww_state_tests.rs
599     shell.rs
265     runtime.rs
231     system.rs
411     admin_tests.rs
333     system_tests.rs
```

**本轮新增/改动的 `.kt`（全部 ≤600）**：`EchoMuseState 552`、`EchoMuseDetailState 531`、`EchoMuseConsoleState 496`、`EchoMuseStatusTab 479`、`EchoMuseConfigStages 470`、`EchoMuseBleState 407`、`EchoMuseConfigDrafts 380`、`EchoMuseConsoleTab 320`、`EchoMuseConsoleText 273`、`EchoMuseUpdatesTab 218`、`EchoMuseConfigTab 209`、`EchoMuseMusicState 204`、`EchoMuseDetailTabs 169`、`EchoMuseBleCard 148`、`EchoMuseDeviceCard 139`、`EchoMuseConsoleProtocol 128`、`EchoMuseLogsState 115`、`EchoMuseMusicPanel 113`、`EchoMuseLogsTab 89`、`EchoMuseActivityTab 86`、`EchoMuseDeviceRow 77`、`AdminFonts 18`（+ 4 个平台 actual 7/7/7/8 行，四端齐全）。

**越 600 行的文件（全部既存，本轮未引入）**：

| 文件 | 本轮前 → 后 | 定性 |
|---|---|---|
| `client/.../ConfigContextState.kt` | 620（未改动） | **既存，本轮未触碰** |
| `server/.../oww.rs` | 712 → 716 | **既存超限**，本轮 +4（新增 `/oww/state` 路由挂载） |
| `server/.../admin.rs` | 626 → 643 | **既存超限**，本轮 +17（白名单注释 + `pub(super)`） |
| `server/.../mod.rs` | 600 → 605 | 本轮**新踩线**（+5：4 个 `mod` 声明 + 注释）→ D4 |

---

## 2. 缺陷清单

> 严重度口径：P0 = 阻断交付；P1 = 主链缺陷/回归；P2 = 文案、文档、测试覆盖、工程约束。

### D1（P2）`closed.reason: "internal"` / `"replaced"` 无专属文案，且 `detail` 被丢弃 → 用户看到误导性原因

- **文件行号**：`client/shared/src/commonMain/kotlin/com/xiaozhi/admin/EchoMuseConsoleState.kt:389-398`（`hint = when (reason)`）、`:376-381`（`onError` 的 `else` 分支）；常量 `EchoMuseConsoleProtocol.kt:60,62` 定义了 `REASON_REPLACED` / `REASON_INTERNAL` 却未进 `when`。
- **现象**（端到端实测，非推测）：设备 15 s 内未回拨时，服务端发
  `{"type":"closed","reason":"internal","detail":"设备未在 15 秒内回拨 shell 通道",…}`；
  前端 `when(reason)` 无 `internal` 分支 → 落 `else` → 显示 `RECONNECT_LOST` =「连接中断（服务端已退出或网络中断）」。**真实原因是设备没回拨，文案却说是服务端退出/网络中断**——用户会去查错误的方向。
- **期望**（设计 §5.2「close reason 汇总」）：`internal` → `控制台内部错误：<detail>`；`replaced` → `会话已被替换`。`detail` 字段当前**前端完全没读**（`grep '"detail"' EchoMuse*.kt` 零命中）。
- **复现**：`--no-shell` 假设备 → 开控制台 → 等 15 s。
- **归属**：**工程师修**（前端 2 处分支 + 读 `detail`）。严重度 P2 的理由：服务端行为与帧字面量都对，纯前端文案映射。

### D2（P2）`docs/echomuse.md:144` 管理台 UI 段落仍描述已删除的左菜单七项，并引用已删除的文件

- **文件行号**：`docs/echomuse.md:144`：「**管理台 UI**…：设备卡左菜单七项——概览…音频调优（`EchoMuseAudioPanel.kt`…）…设备管理、蓝牙代理。」
- **现象**：`EchoMuseAudioPanel.kt` / `EchoMuseManagementCard.kt` / `EchoMuseManagementState.kt` 已删除（`git status` 显示 `D`），左菜单已整体移除（`ECHOMUSE_MENU_` 零命中）。文档却仍按旧结构描述，且引用不存在的文件路径。
- **期望**：改为六 tab（状态/活动/配置/控制台/更新/日志）+ 未批准单页审批，并去掉已删文件名。
- **复现**：`grep -n "左菜单七项\|EchoMuseAudioPanel" docs/echomuse.md` → 命中第 144 行。
- **归属**：**工程师修**（文档属交付物，PRD §2.4/T02 明确要求同步）。

### D3（P2）四条收尾路径仅一条有自动化测试

- **文件行号**：`server/src/device/echomuse/console_tests.rs`（新增前 584 行）
- **现象**：`console.rs` 的 `teardown` 有 4+1 条唤醒路径（浏览器断开 / 设备先断 / control 掉线或被替换 / 定时器 / 兜底 `Drop`），改动前的自动化测试只覆盖「浏览器断开（**已 ready 之后**）」这一条，且 **dial 阶段的浏览器断开零覆盖**（设计 §5.3 把它列为最易漏项）。`device-shell-lost` / `device-offline` / `superseded` / `idle-timeout` / `session-limit` 均无自动化。
- **已处理**：我补了 2 条（dial 阶段关 tab、反向 busy）+ 1 条输入字节边界，放在新文件 `console_path_tests.rs`（不侵占工程师文件的行数预算），**3 条全绿**；`device-shell-lost` / `idle-timeout` 我用端到端实测证明行为正确（§1.5、§1.6），但**未落成自动化测试**。
- **期望**：`device-shell-lost` / `superseded` / `idle-timeout` 三条补成自动化（`idle-timeout` 可用 `tokio::time::pause()` 之类手段，或把 `IDLE_TIMEOUT` 提成可注入参数）。
- **归属**：**工程师修**（源文件的可测性：常量硬编码导致 `idle-timeout` 无法快速验证）／QA 后续补测试。

### D4（P2）`mod.rs` 本轮跨过 600 行；`admin.rs`/`oww.rs` 既存超限又被加长

- **文件行号**：`server/src/device/echomuse/mod.rs` 600 → **605**；`admin.rs` 626 → **643**；`oww.rs` 712 → **716**
- **现象**：仓规「文件 ≤600 行」。`mod.rs` 本轮 +5（4 个 `mod` 声明 + 注释）从 600 变成 605，**新越限**；另两个是既存超限，本轮继续加长（`admin.rs` 的 +17 主要是白名单大段注释，`oww.rs` +4 是路由）。
- **期望**：`mod.rs` 把新增的 4 个 `mod` 声明连同分组注释挪到子模块再导出，或把 `oww.rs` 的 `routes()` 拆出去。
- **归属**：**工程师修**（纯工程整洁度，不影响功能）。

### D5（P2，观察项，可不改）`notice.code` 前端无字面量常量

- **文件行号**：`EchoMuseConsoleProtocol.kt`（`ConsoleWire` 里没有 `output-capped`）
- **现象**：§8.1 把 `notice.code = "output-capped"` 列为共享字面量，服务端有 `NoticeCode::OutputCapped`（serde 派生 → `output-capped`），前端 `NOTICE` 分支只读 `text`、不校验 `code`。
- **影响**：不发散——服务端改码，前端不会坏（也不会有异常表现），只是**少了一个跨端锚点**。若将来新增第二个 notice code，前端将无法区分。
- **归属**：可不改；若改，前端加 `CODE_OUTPUT_CAPPED` 并在服务端 `console_tests` 的字面量断言里一起钉住。

**P0 / P1：0 条。**

---

## 3. 我额外找的 3 个边界（结论）

### 3.1 `\r\n` 混排（原测试只测了裸 `\r`）

- **结论：代码正确，原无测试覆盖 → 我已补测试**。
- 语义分析：`ConsoleBuffer.append` 把 `\r` 当 `cursor = 0`、`\n` 当 `commitLine()`，所以 `"boot ok\r\nready\r\n"` → `"boot ok\nready\n"`（2 行），不会像「把 CRLF 当空行」那样把整段日志吃成空白。
- 我补的两条（`EchoMuseConsoleTextTest.crlf_commits_the_whole_line_instead_of_blank`、`trailing_carriage_return_moves_cursor_without_dropping_content`）**通过**。

### 3.2 空命令回车

- **结论：代码正确（双层防护），服务端行为已钉住**。
- 前端：`EchoMuseConsoleState.submit()` 先 `trimEnd()` 再 `if (line.isEmpty()) return`，且 `canSubmit` 要求 `input.isNotBlank()` → 空行**根本不发**。
- 后端：即使收到 `{"type":"input","data":""}` 也不崩，只写一个 `\n`（`sh` 空命令无副作用）——我把这条写成 `plan_line_counts_utf8_bytes_and_tolerates_empty_line` 的第三个断言（**通过**），避免后人"顺手"改成写空字节（`sh` 在 pipe 模式没有换行会一直攥在行缓冲里）。

### 3.3 超长单行含多字节（`LINE_MAX` 是字节还是字符？）

- **结论：代码正确（按 UTF-8 字节），原测试只用 ASCII → 我已补多字节断言**。
- 后端 `check_line` 用 `data.len()`（字节）；前端 `inputTooLong` 用 `input.encodeToByteArray().size`。两侧一致。
- 我补的断言：1366 个汉字（4098 B）→ `input-too-long`；1365 个（4095 B）→ 放行且总长 4096（含换行）。**通过**。若哪侧误用字符数，中文超长命令会被放过 3 倍——这条断言就是为了钉住它。

### 另外两个我顺手核过但**没**写测试的边界

- **`closed` 之后用户继续输入**：前端 `submit()`/`interrupt()` 首行判 `phase != READY` 直接 return；后端 `teardown` 置 `closed` 后 `pump` 立即返回，不会再写设备。**代码正确，无测试**。
- **连续两次点「重新连接」是否会开出两条 shell**：`open()` 有 `sameDevice && (phase == CONNECTING || phase == READY)` 幂等短路（`EchoMuseConsoleState.kt:176-179`），第二次点击直接返回；即使真开出第二条，服务端也只有一把槽 → 第二条拿 `error{busy}`。**代码正确，无测试**（且有服务端兜底，风险低）。

---

## 4. 我对测试代码做的改动（QA 自己的部分）

1. **新增** `server/src/device/echomuse/console_path_tests.rs`（233 行，`console.rs` 里以 `#[path] mod path_tests` 挂载）：
   - `browser_close_during_dial_releases_slot_without_waiting_for_dial_timeout`
   - `console_gets_busy_frame_when_one_shot_holds_slot`
   - `plan_line_counts_utf8_bytes_and_tolerates_empty_line`
   - 为什么单独成文件：并进 `console_tests.rs` 会把它从 584 顶到 697，**违反 ≤600 硬约束**（我第一版就是这么干的，已改）。
2. **新增** 2 条 Kotlin 用例到 `EchoMuseConsoleTextTest.kt`：`crlf_commits_the_whole_line_instead_of_blank`、`trailing_carriage_return_moves_cursor_without_dropping_content`（该文件 207 → 231 行，仍远低于 600）。
3. **未改动任何源码**（`console.rs` 只加了 3 行模块声明 + 注释，共 531 → 537）。

复跑结果：`console::` 14 passed；`adminCoreTests` 72 passed / 0 failed；`cargo fmt --check` 干净。

---

## 5. 未验证项（明确列出，不含糊成"通过"）

| 项 | 原因 | 建议的验证方式 |
|---|---|---|
| `session-limit`（7200 s 硬上限）收尾 | 需真实等待 2 小时；`SESSION_MAX` 是硬编码常量，无法在测试里缩短 | 有真机/长跑环境时挂一条 2 小时会话观察 `closed{reason:"session-limit"}`；或把 `SESSION_MAX` 提成可注入参数后写 `tokio::time::pause()` 测试。**注意**：`idle-timeout`（600 s）我已真实等到并验证（§1.6），但那次会话同时也是"未到 7200 s"，所以 `session-limit` 分支本身仍未被任何一次运行命中 |
| `superseded`（设备重拨替换 hub 条目） | 需构造「旧 control 通道仍活、hub 条目已被新 register 替换」的窗口；mock 重连会先让旧通道 `is_closed()` 为真，实际会走 `device-offline` | 在服务端注入一次「替换 hub 但保留旧 ctl」的测试装置（`hub_insert` 两次，中间不 drop 旧 ctl） |
| `device-offline`（control 通道断开） | 需中途 kill 掉设备的 control 连接（mock 是单进程内两条 socket，kill 会连带 data 面） | 单独 socket 级别的断链测试，或 `iptables`/代理切断 control |
| `16 MiB` 会话封顶的**端到端**表现（`notice` 只一次、会话不断） | 需向设备灌 16 MiB 输出；mock 假 shell 没有大输出命令 | 纯函数已 100% 钉住（`budget_caps_once_without_ending_session`）；端到端可在 mock 里加一个 `dmesg` 生成器后补 |
| 浏览器（H5/真实 Kuikly 渲染）手测 | 本机未发布 h5App 产物、无浏览器交互环境；本轮所有 UI 结论来自**源码走查 + 四端编译** | 在 `client` 里 `:apps:h5App:publishWeb` 后按 PRD §8.1 的六条验收口径手测（≤3 次交互 / 断网文案 / busy 文案 / 10 分钟超时） |
| Android 端 `unsupported` 退化 | 需要 Android 宿主；仅源码走查（`PLATFORM_WS_SUPPORTED=false` → 不建连、输入行整块不渲染） | Android 真机点控制台，确认文案 `当前平台暂不支持控制台（Android 客户端未接入 WebSocket）` 且无循环重试 |
| 六 tab 在超窄窗的视觉裁字（设计 §11 R5） | 需真机/模拟器量宽 | 手机横屏分屏截图核对 |

---

## 6. 交付建议

1. **可以合入**：主链端到端成立、帧协议零漂移、四端编译 + 单测全绿、既有一次性命令路径无回归。
2. **建议同批修 D2（文档 144 行）**——它是唯一「文档与代码互相打脸」的一条，最容易误导下一个人。
3. **D1 建议在下一个提交修**（前端两处 `when` 分支 + 读 `detail`，10 行内），因为 `internal` 在实际运行中确实会发生（dial 超时 15 s 是常见路径），文案会把人引向错误方向。
4. **D3/D4 记入技术债**：`device-shell-lost`/`superseded`/`idle-timeout` 补自动化；`mod.rs` 回落 600 行以内并把 `oww.rs`/`admin.rs` 的既有超限拆掉。
5. 端到端脚本（`/tmp/qa/console_probe*.py`、`idle_probe.py`）建议收进 `tools/`（配合 mock 的 `--stay`），让「无真机也能验控制台」成为可重复能力——这次正是靠它拿到了最硬的证据。
