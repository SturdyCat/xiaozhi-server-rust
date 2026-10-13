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

`src/` 目录本身不再单独维护文档，以下直接跨级进入其子目录：

- [配置](<src/config/AGENTS.md>)：来源、兼容迁移、ConfigStore。
- [引擎装配](<src/engine.rs>)：共享实例、VAD 池、CPU 预算与热刷新。
- [应用层](<src/app/AGENTS.md>)、[插件层](<src/plugins/AGENTS.md>)：按各自范围继续向下阅读。
- [测试说明](<tests/AGENTS.md>)、[部署说明](<../docs/deployment.md>)：联调、模型、镜像与挂载。

## 上级入口

[上级 AGENTS](<../AGENTS.md>)

## 直属下级

- [app](<src/app/AGENTS.md>)
- [config](<src/config/AGENTS.md>)
- [plugins](<src/plugins/AGENTS.md>)
- [tests](<tests/AGENTS.md>)
