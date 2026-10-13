# 配置 HTTP 契约

[端点实现](<../config.rs>)负责请求处理，[impact](<impact.rs>)计算保存影响；存储抽象见[配置模块（从上级入口导航）](<../AGENTS.md>)。

- GET 永不返回密钥明文，只回 `has_<field>` presence 与 revision。缺失/空串密钥=保留，显式 null=清除，非空=覆盖。
- 新密钥字段必须同批改服务端 SECRET_PATHS、客户端 fill presence 与 putSecret 写入。presence/revision 包装字段不能持久化；revision 忽略密钥值但跟踪有无。
- 部分更新未提供字段保持原值，显式 null 回落默认；旧客户端扁平字段补丁先重写到规范位置，同补丁显式新位置优先。
- expected_revision 不一致返回 409；客户端保留草稿并提示重新加载，不能静默覆盖。
- 写入统一走 ConfigStore，应用预设复用相同合并、校验、密钥与 hot 回报管线，不另造保存路径。
- hot 判定先 normalize 再叶子 diff，字段级优先于段级，多项取最严格档位；规则只从[注册表（从上级入口导航）](<../AGENTS.md>)读。
- overlay 只允许会话内白名单字段，不能改引擎选择/模型/网络/音频结构；非法补丁拒绝，存量读取裁剪越界字段，全局引擎不被 overlay 重建。

回归：[密钥](<tests/secrets.rs>)、[旧客户端](<tests/legacy.rs>)、[影响判定](<tests/impact.rs>)、[预设](<tests/presets.rs>)及[端点总测试](<tests.rs>)。

## 上级入口

[上级 AGENTS](<../AGENTS.md>)
