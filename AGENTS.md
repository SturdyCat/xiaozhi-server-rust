# 仓库核心指引

小智 ESP32 语音服务端（Rust）与 React 管理台（`client/admin-web`）。先沿下级入口阅读目标模块指引、源码说明和测试。

## 全局约定

- 不自动 `git add` / `git commit`，不覆盖任务无关的工作区变更。
- `.rs` / `.kt` 逻辑文件不超过 600 行，按职责拆分；Rust 测试外置。纯搬移须验证行为不变。
- 协议保持 ESP 固件兼容；配置热生效档位由注册表元数据唯一声明。
- **维护指引统一使用大写 AGENTS.md，根目录 README.md 保留为项目介绍与使用入口**：各级只维护本级职责与约束，可直接跨多级引用相关说明，不重复堆积子模块细节。
- 仅仓库根目录保留 `docs/`：普通 Markdown（含长文）只允许出现在根 `docs/` 与根 `README.md`，其他目录一律只保留该级 `AGENTS.md`（技能库、第三方依赖、构建产物与**插件源交付面 `plugins-src/*/README.md`** 除外——后者由 `plugins/plugin_docs_tests.rs` 结构检查强制，属显式豁免）。同一范围的说明与长文合计不超过 400 行时写在一个文档里；更长的设计、历史与跨模块记录统一归档到根 `docs/`，由相关 `AGENTS.md` 直接引用。不建模块 `docs/`，不写只有“见链接”的纯导航文件；当前事实以源码、测试与本级说明为准，修改行为同步更新文档。
- 技能库、第三方依赖和构建产物按自身格式维护，不纳入项目说明改名。

## 直属下级

- [server](<server/AGENTS.md>)：服务端构建、依赖、源码与测试。
- [client](<client/AGENTS.md>)：客户端平台、构建、React 管理台（`client/admin-web`）与旧端原生宿主归档。
- [plugins-src](<plugins-src/AGENTS.md>)：15 个官方系统插件源码根（契约 `xiaozhi:plugin@0.2.0`；产物由 `tools/build-system-plugins.sh` 构建到 `build/system-plugins/`，共 16 个 `.wasm`）。

## 本级资料

[文档索引](<docs/README.md>)包含项目介绍、当前架构、部署及跨模块计划；模块设计从各自下级入口进入。
