# 设备管理指引

继承[服务端指引](<../../AGENTS.md>)。设备管理：EchoMuse 设备控制面、固件 OTA、发音人目录、Tuya IoT、局域网管理客户端发现、进程内设备/会话注册表。

- EchoMuse 见[EchoMuse 入口](<echomuse/AGENTS.md>)：BLE/WS 控制面直连 emOS；协议权威在 EchoMuse 仓库 `docs/device-controller-interface.md`（以代码为准），本侧实现以源码与测试为准；注册表已登记 `echomuse: RestartOnly`（护栏测试会抓遗漏）。
- 固件托管见[OTA 说明](<firmware/AGENTS.md>)：只回 `websocket` 字段，**有意不下发 `mqtt`**（正确克制，不是缺口）。
- 音色见[音色探测](<voices/AGENTS.md>)：发音人目录探测，固定并发 2。
- Tuya IoT `tuya/`：涂鸦云目录同步（官方 v1.0 签名 + 授权设备清单，2026-11-03 真凭证联调验证，模块头注释为设计权威）+ 纯局域网发现/DP 控制 + 管理面 `/api/tuya/*`（access_secret 只写不读）；云调用调试代理 `POST /api/tuya/cloud/proxy`（签名转发、回包透传）；凭证/区域/授权诊断用 `tools/tuya_probe.py`。
- **外部 SDK 集成约定**（2026-11-03 起）：第三方云 SDK 一律封装为 Rust 客户端 + 管理面代理端点（服务端负责签名/转发、回包原样透传、错误码三归类），联调、调逻辑、修 bug 都打本服务端代理，不写临时脚本、不把凭证外泄到工具层；签名口径与黄金向量进 Rust 测试钉死。
- 局域网发现 `discovery.rs`：常驻 mDNS `_xiaozhi-admin._tcp` 广播，macApp 免手填 IP；发现是辅助，失败只告警、不阻断。
- 进程内设备/会话/用量注册 `registry/mod.rs`：重启清零，长期对话由记忆留存。

## 上级入口

[服务端指引](<../../AGENTS.md>)

## 直属下级

- [EchoMuse](<echomuse/AGENTS.md>) `echomuse/`
- [固件 OTA](<firmware/AGENTS.md>) `firmware/`
- [音色](<voices/AGENTS.md>) `voices/`
- [Tuya IoT](<tuya/mod.rs>) `tuya/`
- [发现](<discovery.rs>) `discovery.rs`
- [设备注册表](<registry/mod.rs>) `registry/`
