# 设备 MCP 契约

设备是 MCP Server，语音服务器是 MCP Client；JSON-RPC 2.0 封在既有 WS 的 `type=mcp` 信封中，不新建连接。

- 纯协议原语复用[MCP 模块](<./mod.rs>)，I/O 与响应关联复用[会话 MCP](<../session/mcp.rs>)，不要各处手拼 JSON。
- id 必须是数字，jsonrpc 必须为 2.0；畸形响应不得当成功。`-32602 Unknown tool` 是设备拒绝响应，不是传输失败。
- 未声明 `features.mcp` 时一个请求也不发。stop 工具名以 `STOP_LISTENING_TOOL` 与固件注册名的跨仓库契约为准。
- [能力发现](<discovery.rs>)在主循环之前做 initialize + tools/list，全部分页共用预算；探测中的音频必须缓存后按原序补派发，不能多读或丢掉第一句话。
- 测试台不做发现。失败非致命，[设备工具注册表](<registry.rs>)保留上次成功清单，并显式报告拒绝/超时/断开原因。
- `annotations.audience=["user"]` 是 user_only 的依据；user_only 不暴露给 LLM，但仍可按名字直接调用。分页 cursor 遵守固件“包含下一页首工具”的语义。
- 指令 stop 决策由工具清单三态驱动：已知支持才调用，已知不支持直接降级，未知允许试探。发现完成不等于 LLM 工具闭环已实现。
- 主循环内调用用同任务抽帧；独立 task 的调用才能由主循环通过 pending/oneshot 投递，不能互换。

回归：[协议测试](<tests.rs>)、[发现测试](<discovery/tests.rs>)、[注册表测试](<registry/tests.rs>)、[会话抽帧测试](<../session/mcp/tests.rs>)。

## 上级入口

[上级 AGENTS](<../AGENTS.md>)
