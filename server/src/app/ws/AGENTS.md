# HTTP 与 WebSocket 网关

[路由表](<mod.rs>)是端点注册入口；鉴权与握手也在此，生产会话交给 session。管理台 UI 不维护另一套路由语义。

- 协议与 hello 见[协议说明（从上级入口导航）](<../AGENTS.md>)；下行版本跟随设备 hello，而不是服务端配置独立选版本。
- 配置 GET 打码、PUT/POST 部分更新、revision 冲突与热生效回报，见[配置端点说明](<config/AGENTS.md>)。
- 设备/会话/最近对话/用量/延迟是进程内观测态，重启清零；online 来自生命周期，不是心跳，也不是“今日统计”。长期原文留存属于 memory。
- 健康接口与插件清单共用宿主 capability 聚合，不另写 fold；探测只做本地检查，不构建引擎或请求外网。
- Kuikly NetworkModule 只用 GET/POST；保留配置 POST=PUT、overlay/preset 删除 POST 别名，不能只测 curl 的 PUT/DELETE。
- 人格 preview 接受未保存草稿，返回完整 effective；记忆 recall 是当场验证入口，clear 必须显式 scope 或 all。
- 固件使用[固件子路由](<../firmware/http.rs>)，按功能/型号匹配；不要把统一插件 action/job 设计当成已存在路由。

当前接口以路由与测试为准；管理操作见[管理台用法（从上级入口导航）](<../AGENTS.md>)。

## 上级入口

[上级 AGENTS](<../AGENTS.md>)

## 直属下级

- [config](<config/AGENTS.md>)
