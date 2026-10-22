# 节点测试服务指引

继承[服务端指引](<../../AGENTS.md>)。统一、快照隔离的节点测试服务（原 `app/bench_api`）：每请求取一份保存态 SQLite 快照 + 字符串 revision，不从启动配置或磁盘 TOML 回退。路由经 [gateway/ws/bench_api.rs](<../gateway/ws/bench_api.rs>) 挂载，与其它接口共用同一管理鉴权，不因"只是测试"放开模型资源。

- **隔离边界**：不发布生产引擎、不写生产会话 / 历史 / 记忆 / 设备动作 / 音频缓存；默认 feature 不把音频编译桩当真实 provider。
- `asr_test` / `tts_test` / `llm_test` 仅 `hello.test=true` 会话受理，且经 [推理闸门](<../workflow/pipeline/inference/AGENTS.md>)。
- 能力只读投影（`capabilities.rs`）、无副作用本地流水线诊断（`pipeline.rs`）、隔离真实 provider 工厂（`providers.rs`）；wire 契约只增不改，**绝不序列化配置或凭据**（`contracts.rs`）。
- 样本为闭集内嵌目录（`samples.rs`），不接受来自请求的文件路径或远程 URL；精确上限、请求形状与样本来源见[契约实现](<contracts.rs>)与 [node-test-samples](<../../../docs/node-test-samples.md>)。
- 测试不证明原生模型可加载、真实 TTS 音色可用、设备收到数据或扬声器播放。

## 上级入口

[服务端指引](<../../AGENTS.md>)

## 直属下级

- 契约 `contracts.rs`
- 能力投影 `capabilities.rs`
- 模型 `models.rs`
- 流水线诊断 `pipeline.rs`
- provider 工厂 `providers.rs`
- 样本 `samples.rs`
- 特性 / 预览 `features.rs`
- 回归测试 `tests/`
