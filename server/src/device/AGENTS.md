# 设备管理指引

继承[服务端指引](<../../AGENTS.md>)。设备管理：EchoMuse 设备控制面、固件 OTA、发音人目录、Tuya IoT、局域网管理客户端发现、进程内设备/会话注册表。

- EchoMuse 见[EchoMuse 入口](<echomuse/AGENTS.md>)：BLE/WS 控制面直连 emOS；协议权威在 EchoMuse 仓库 `docs/device-controller-interface.md`（以代码为准），本侧实现以源码与测试为准；注册表已登记 `echomuse: RestartOnly`（护栏测试会抓遗漏）。
- 固件托管见[OTA 说明](<firmware/AGENTS.md>)：只回 `websocket` 字段，**有意不下发 `mqtt`**（正确克制，不是缺口）。
- 音色见[音色探测](<voices/AGENTS.md>)：发音人目录探测，固定并发 2。
- Tuya IoT `tuya/`：涂鸦云目录同步（官方 v1.0 签名 + 授权设备清单，2026-11-03 真凭证联调验证，模块头注释为设计权威）+ 纯局域网发现/DP 控制 + 管理面 `/api/tuya/*`（access_secret 只写不读）；云调用调试代理 `POST /api/tuya/cloud/proxy`（签名转发、回包透传）；凭证/区域/授权诊断用 `tools/tuya_probe.py`。签名/请求构造等**纯协议语义**收敛在 `tuya/protocol.rs` 单源（guest 经 `#[path]` 共用，`plugins/wasm/tests/protocol.rs` 与宿主黄金向量逐字节对拍）；DP 载荷解析收敛在 `tuya/dps.rs` 单源（native `lan` 数据面与 guest 插件共用，`dps_tests` 钉住两种报文形状）；secret 仍永不跨 WASM 边界，生产签名调用方是持有凭证的 native `CloudClient`。
- **外部 SDK 集成约定**（2026-11-03 起）：第三方云 SDK 一律封装为 Rust 客户端 + 管理面代理端点（服务端负责签名/转发、回包原样透传、错误码三归类），联调、调逻辑、修 bug 都打本服务端代理，不写临时脚本、不把凭证外泄到工具层；签名口径与黄金向量进 Rust 测试钉死。
- 局域网发现 `discovery.rs`：常驻 mDNS `_xiaozhi-admin._tcp` 广播，macApp 免手填 IP；发现是辅助，失败只告警、不阻断。
- 进程内设备/会话/用量注册 `registry/mod.rs`：重启清零，长期对话由记忆留存。
- **设备公共能力层** `capabilities/`：承载跨 EchoMuse/Xiaozhi ESP/ESPHome 复用的 capability ID、参数边界和纯 helper；业务技能/插件复用此层，不能复制音量、生命周期、状态逻辑。PCM、VAD、推理、WS/keepalive、PTY、Tuya LAN worker 等 hot path 保持 native，WASM 仅可选用于低频 control path。

## 上级入口

[服务端指引](<../../AGENTS.md>)

## 直属下级

- [EchoMuse](<echomuse/AGENTS.md>) `echomuse/`
- [ESPHome 桥](<esp_home/mod.rs>) `esp_home/`：把 Dot 2 暴露为标准 ESPHome 节点（`--features esp_home`，默认关闭）；`[esp_home].engine = "wasm-policy"` 时实体目录放行/改名经 `connector.esp-home` 插件决策（fail-closed，数据面留 native）。
- [固件 OTA](<firmware/AGENTS.md>) `firmware/`
- [音色](<voices/AGENTS.md>) `voices/`
- [Tuya IoT](<tuya/mod.rs>) `tuya/`
- [发现](<discovery.rs>) `discovery.rs`
- [设备注册表](<registry/mod.rs>) `registry/`
