# 设备管理指引

继承[服务端指引](<../../AGENTS.md>)。设备管理：EchoMuse 设备控制面、固件 OTA、发音人目录、局域网管理客户端发现、进程内设备/会话注册表。

- EchoMuse 见[EchoMuse 入口](<echomuse/AGENTS.md>)：BLE/WS 控制面直连 emOS；协议权威在 EchoMuse 仓库 `docs/device-controller-interface.md`（以代码为准），本侧实现以源码与测试为准；注册表已登记 `echomuse: RestartOnly`（护栏测试会抓遗漏）。
- 固件托管见[OTA 说明](<firmware/AGENTS.md>)：只回 `websocket` 字段，**有意不下发 `mqtt`**（正确克制，不是缺口）。
- 音色见[音色探测](<voices/AGENTS.md>)：发音人目录探测，固定并发 2。
- 局域网发现 `discovery.rs`：常驻 mDNS `_xiaozhi-admin._tcp` 广播，macApp 免手填 IP；发现是辅助，失败只告警、不阻断。
- 进程内设备/会话/用量注册 `registry/mod.rs`：重启清零，长期对话由记忆留存。

## 上级入口

[服务端指引](<../../AGENTS.md>)

## 直属下级

- [EchoMuse](<echomuse/AGENTS.md>) `echomuse/`
- [固件 OTA](<firmware/AGENTS.md>) `firmware/`
- [音色](<voices/AGENTS.md>) `voices/`
- [发现](<discovery.rs>) `discovery.rs`
- [设备注册表](<registry/mod.rs>) `registry/`
