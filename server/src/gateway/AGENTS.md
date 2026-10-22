# 网关接口指引

继承[服务端指引](<../../AGENTS.md>)。管理后台 / 网关接口：WebSocket 网关、配置与观测 HTTP 端点、静态文件托管、SSE 推送。

- WS 网关见[网关说明](<ws/AGENTS.md>)：路由、鉴权、协商；配置更新走唯一合并 / 打码 / 持久化管线，热生效规则来自注册表，不在文档复制热生效表。
- 配置端点见[HTTP 契约](<ws/config/AGENTS.md>)：部分更新（缺失=保持 / 显式 `null`=清除）、密钥 presence、`revision` + 409 乐观并发；设备 overlay 只覆盖白名单会话参数，不重建全局引擎。
- 文件管理见[文件管理](<files/AGENTS.md>)：数据目录文件管理；`oww-models/` 为唤醒词模型上传目录。
- 观测接口（metrics / usage / memory / health）与节点测试路由同挂本层，共用同一管理鉴权，不因"只是观测/测试"放开资源。
- SSE `sse.rs`：日志 / 事件推送；时间格式由 [observability/logtime.rs](<../observability/logtime.rs>) 唯一出口。

## 上级入口

[服务端指引](<../../AGENTS.md>)

## 直属下级

- [WS 网关](<ws/AGENTS.md>) `ws/`（含 [配置端点](<ws/config/AGENTS.md>)）
- [文件管理](<files/AGENTS.md>) `files/`
- [SSE](<sse.rs>) `sse.rs`
