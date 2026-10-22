# Rust 服务端指引

继承[根指引](<../AGENTS.md>)。构建与依赖版本以[Cargo 清单](<Cargo.toml>)、[锁文件](<Cargo.lock>)为准，不在文档维护第二份版本表。

## 验证

以下命令在仓库根执行：

```bash
~/.cargo/bin/cargo test --manifest-path server/Cargo.toml
DOCS_RS=1 ~/.cargo/bin/cargo check --manifest-path server/Cargo.toml --features sherpa
```

- cargo 不在 PATH 时用上述路径；USTC 镜像报 404 时追加 `--config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'`，不要假定每台机器都需要镜像覆盖。
- 默认 feature 仅用于编译/单测；完整服务运行需要 `sherpa`、原生库与模型，不能把单测桩称为可运行 mock 服务。
- `DOCS_RS=1` 只证明 feature 代码可类型检查，不代表链接成功或真实推理可用。真实环境另跑 `cargo test --features sherpa` 和协议联调。
- macOS 编译真实 feature：安装 Homebrew `opus`，设置 `PATH="/opt/homebrew/bin:$PATH"` 与 `PKG_CONFIG_PATH="/opt/homebrew/lib/pkgconfig:$PKG_CONFIG_PATH"`。找不到系统 libopus 会回退 autotools，缺 automake/aclocal 时编译失败；本地动态链接产物不是 Docker 部署产物。
- Rust 测试外置；子模块可访问父模块私有项，父模块调用子模块函数仅按需用 `pub(super)`，不因拆文件扩大为 `pub`。

## 逻辑入口

`src/` 按业务功能模块组织（2026-10-22 由 `app/` 技术伞拆出顶层模块，消除平铺）。以下跨级进入各模块与其子文档：

- [配置](<src/config/AGENTS.md>)：来源、兼容迁移、ConfigStore。
- [运行期装配](<src/runtime/mod.rs>)（原 `engine.rs`）：共享实例、VAD 池、CPU 预算与热刷新。
- [工作流](<src/workflow/AGENTS.md>)：会话 / 流水线 / 协议 / 下行 / MCP 的编排与边界。
- [设备](<src/device/AGENTS.md>)：EchoMuse 直连、固件 OTA、音色、发现、设备注册表。
- [网关](<src/gateway/AGENTS.md>)：WS 网关、配置 / 观测 HTTP、文件托管、SSE。
- [评测](<src/benchmark/AGENTS.md>)：节点测试服务与样本目录（原 `app/bench_api`）。
- [媒体](<src/media/AGENTS.md>)：Opus / 重采样 / PCM / 下行预算 / 热欢迎语（原 `app/audio` + `app/hot_audio`）。
- [观测](<src/observability/AGENTS.md>)：指标 / 事件 / 用量 / 时间格式。
- [插件层](<src/plugins/AGENTS.md>)：按能力分目录（asr/tts/llm/vad/nlu/memory/soul/skill/command/wakeup/prompt/aiui/health/host/registry）。
- [测试](<tests/AGENTS.md>)、[部署](<../docs/deployment.md>)：联调、模型、镜像与挂载。

## 上级入口

[上级 AGENTS](<../AGENTS.md>)

## 直属下级

- [workflow](<src/workflow/AGENTS.md>)
- [device](<src/device/AGENTS.md>)
- [gateway](<src/gateway/AGENTS.md>)
- [config](<src/config/AGENTS.md>)
- [benchmark](<src/benchmark/AGENTS.md>)
- [media](<src/media/AGENTS.md>)
- [observability](<src/observability/AGENTS.md>)
- [runtime](<src/runtime/mod.rs>)
- [plugins](<src/plugins/AGENTS.md>)
- [tests](<tests/AGENTS.md>)
