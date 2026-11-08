# 插件源根（plugins-src/）

真实 WASM 插件的源码根（方案 v4）。**用户插件不编译进主系统，wasm 产物一律不入 git**——源码是唯一入库物；生产装载走管理界面上传已构建组件（`POST /api/plugins/upload`）或镜像随附的官方系统插件目录。官方系统插件由 [tools/build-system-plugins.sh](<../tools/build-system-plugins.sh>) 按本表构建，运行时由 WASM boot 自动导入、注册并激活；用户插件仍由数据卷生命周期管理。guest 统一经 [xiaozhi-plugin-sdk](<../server/crates/xiaozhi-plugin-sdk/AGENTS.md>) 使用契约；测试夹具性质的演示插件在 `tools/wasm-fixtures/`（系统级 hello 探针暂复用该源码验证全链路）。

## 直属下级

- [weather](<weather/AGENTS.md>)：NMC 天气官方默认插件（P1c 试点）。

## 约定

- **每个插件目录必须自带 `README.md`**（功能描述与使用方法是插件的交付面，不是可选项）：
  必含 `## 功能描述` 与 `## 使用方法` 两节——事件/payload、动作、配置字段、kv 键布局、
  权限与扩展点、边界限制都要写清。结构由 `plugins/wasm/plugin_docs_tests.rs`
  强制检查（新增插件缺文档 = 测试失败）。测试件（tools/wasm-fixtures）不在此列。
- 插件 id 形如 `<域>.<名>`（`weather.official`），manifest 声明实现的扩展点与权限；宿主校验见 `plugins/wasm/contract.rs`。
- 与原生实现共享纯逻辑源时用 `#[path]` 单源引用（绞杀者方向：P8 反转为原生从这里取）。
- 多步 http 流程的上下文编码在 request-id（`|` 分隔）；全站级大列表不进 kv（值上限 64KB）。
