# 应用层指引

继承[服务端指引](<../../AGENTS.md>)。应用层编排协议、承载、会话和 HTTP，不嵌入具体引擎实现。

- 改协议先读[协议说明](<protocol/AGENTS.md>)与[协议实现](<protocol.rs>)，验证 serde 标签、网络序及 hello 协商，并做联调。
- 改音频先读[下行说明](<downlink/AGENTS.md>)；不得删除 pacing、逐句预取或背压，也不得在句间重置回复节拍。
- 改会话先读[会话说明](<session/AGENTS.md>)；推理许可、abort、MCP 抽帧与缓存路径均有独立边界。
- 改 HTTP 先读[网关说明](<ws/AGENTS.md>)；配置更新继续走唯一合并/打码/持久化管线，热生效规则来自注册表。
- 固件托管见[OTA 说明](<firmware/AGENTS.md>)，发音人目录见[音色说明](<voices/AGENTS.md>)，EchoMuse 设备直连见[EchoMuse 说明](<echomuse/AGENTS.md>)（协议映射见[根文档](<../../../docs/echomuse.md>)）。
- 管理客户端局域网自动发现（常驻 mDNS `_xiaozhi-admin._tcp` 广播，macApp 免手填 IP）见 [discovery.rs](<discovery.rs>)；发现是辅助，失败只告警。

跨层结构见[架构概览](<../../../docs/architecture.md>)；历史三项缺口设计、接线清单与实施护栏见[三项缺口设计](<../../../docs/gap-implementation-design.md>)，不作为现状规范。

## 上级入口

[上级 AGENTS](<../../AGENTS.md>)

## 直属下级

- [downlink](<downlink/AGENTS.md>)
- [echomuse](<echomuse/AGENTS.md>)
- [firmware](<firmware/AGENTS.md>)
- [inference](<inference/AGENTS.md>)
- [mcp](<mcp/AGENTS.md>)
- [metrics](<metrics/AGENTS.md>)
- [protocol](<protocol/AGENTS.md>)
- [session](<session/AGENTS.md>)
- [voices](<voices/AGENTS.md>)
- [ws](<ws/AGENTS.md>)
