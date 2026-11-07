# guest SDK（xiaozhi-plugin-sdk）

小智 WASM 插件的 guest 侧 SDK：契约绑定唯一入口 + 运行期分配纪律 + 静态注册构建器。**独立 crate，不进 server 依赖图**（宿主永不依赖 guest 代码，依赖单向：插件→契约→宿主）。设计见根 [WASM 插件平台](<../../../docs/designs/wasm-plugin-platform.md>)。

## 结构与纪律

- `src/lib.rs`：`wit_bindgen::generate!` 指向 `server/wit/plugin.wit`（唯一契约源）；guest `use xiaozhi_plugin_sdk::*` + `xiaozhi_plugin_sdk::export!(Component);`。
- `src/fallible.rs`：**运行期载荷**的可失败分配（`try_*`：分配失败 → 显式 `Err`，业务性失败不降级；panic/abort 才是陷阱 → Degraded）。
- `src/builder.rs`：**静态注册包**（manifest/菜单/字段/动作）的类型化构建；校验规则与宿主 `plugins/wasm/contract.rs::validate_*` 同源，规则变更双侧同步。静态数据用 expect 语义（失败=激活期 `Failed`）。
- 测试外置（`tests/builders.rs`，native 单元级）；guest 全链路由 `tools/wasm-fixtures` 真实组件与 server `wasm` 测试覆盖。

## 验证与契约同步

```bash
~/.cargo/bin/cargo test --manifest-path server/crates/xiaozhi-plugin-sdk/Cargo.toml
./tools/wasm-fixtures/build.sh   # 改契约后必须重建全部夹具
```

改 `server/wit/plugin.wit` → 重编本 crate → 重建夹具（`tools/wasm-fixtures/AGENTS.md` 的坑表）。
