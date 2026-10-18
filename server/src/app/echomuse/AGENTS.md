# EchoMuse 设备入口

继承[应用层指引](<../AGENTS.md>)。Echo Dot 2（emOS）经 EchoMuse 设备协议直连的**第二设备承载**，与 xiaozhi WS 共用同一条会话流水线；设计、协议映射表与不做范围见[根文档](<../../../../docs/echomuse.md>)，此处只列维护约束。

- 文件分工：[`frames.rs`](<frames.rs>) 是 `/em/data` 二进制帧的唯一编解码出口（帧型按方向分命名空间，不在此文件之外解析）；[`transport.rs`](<transport.rs>) 实现 `Transport`（只做翻译，PCM↔Opus 用 `app::audio` 同一套编解码器，不另写）；[`routes.rs`](<routes.rs>) 是两条 WS 路由 + 只读状态查询 `GET /em/status`（关闭时也 200 + `enabled:false`，devices 取 hub key）+ control/data 两个 inbound 任务 + hub 认领；[`registry.rs`](<registry.rs>) 把 register 的静态元数据 upsert 到数据目录 `echomuse_devices.json`（label/approved/版本/首末见时间落盘，online 由 hub 实时合并不落盘）；[`admin.rs`](<admin.rs>) 是 `/api/echomuse/*` 管理面（经 `auth_ok` 鉴权，enabled=false 全 404；列表/详情/认领/下发 command/每设备 settings，command 在线 202 经 `PendingDevice.ctl_tx` 下发、离线 409，settings 白名单过滤后持久化到 registry 并在线即时推送/离线待接入推送；API 细节见[根文档](<../../../../docs/echomuse.md>)管理面一节）；[`mod.rs`](<mod.rs>) 持 `[echomuse]` 配置、hub、ack/config 文本（`config_push_with` 合并每设备 settings，`wakeWordEnabled` 单独成条）与 mDNS 广播。
- 会话语义映射以根文档表格为准：唤醒/按键 → `mic_start{lock_mic:true}` + `listen start`，`0x04/0x05` 哨兵 → `listen stop`，`tts stop` → `0x03`。改映射先读 EchoMuse `device-controller-interface.md` 再改 `routes.rs`，同步根文档与 `tests.rs`。
- 能力协商是兼容性契约：`ack.features` 只声明服务器支持的子集；不声明的能力（`listen_session`/`audio_mix`/`ble_connect`）靠协议"未声明即降级"关闭，**不要**解析对应帧型。
- 会话无 xiaozhi hello：参数由 `session_params` 固定构造（16k/v1/60ms），改了要连 `transport.rs` 的编解码采样率一起核。
- `[echomuse]` 属 RestartOnly，已登记 `registry::APP_SECTION_HOT`（护栏测试会抓遗漏）；改动本段需重启。
- 测试外置在 [`tests.rs`](<tests.rs>)（帧编解码/协商形状/transport 翻译/`/em/status` 状态形状/registry 落盘往返/管理面响应决策，channel 直驱不起真实 WS）；真机联调用 `tools/echomuse_mock_device.py`。

## 上级入口

[上级 AGENTS](<../AGENTS.md>)
