# guest SDK（xiaozhi-plugin-sdk）

小智 WASM 插件的 guest 侧 SDK：契约绑定唯一入口 + 运行期分配纪律 + 类型化注册构建器。**独立 crate，不进 server 依赖图**（宿主永不依赖 guest 代码，依赖单向：插件 → 契约 → 宿主）。当前契约 `xiaozhi:plugin@0.2.0`，权威说明见 [plugins/wasm/AGENTS.md](<../../../plugins/wasm/AGENTS.md>)；[方案 v4 设计长文](<../../../docs/designs/wasm-plugin-platform.md>) 属 v0.1 历史记录。

## 结构与纪律

- `src/lib.rs`：`wit_bindgen::generate!` 指向 `server/wit/plugin.wit`（唯一契约源）；`API_VERSION = "0.2"`，与宿主 `CONTRACT_MAJOR_VERSION=0` / `CONTRACT_MINOR_VERSION=2` 必须一致。guest `use xiaozhi_plugin_sdk::*` + `xiaozhi_plugin_sdk::export!(Component);`。
- `src/fallible.rs`：**运行期载荷**的可失败分配（`try_*`：分配失败 → 显式 `Err`，业务性失败不降级；panic/abort 才是陷阱 → Degraded）。
- `src/builder.rs`：manifest / 注册包的类型化构建。0.2 新增 `subscribe / endpoint / resources / provide / service`；`config_namespace` 恒等于插件 id；菜单槽位由白名单改为**形状校验**（`<域>.<名>`、≤64 字节），宿主不认识业务域；配置字段类型为 WIT `config-field`（旧 `field-schema` 已更名）。校验规则与宿主 `plugins/wasm/contract/validate.rs` 同源（manifest / registration / endpoint / subscription），规则变更双侧同步。静态数据用 expect 语义（失败 = 激活期 `Failed`）。
- 0.2 guest 必须实现四个导入面所需的实现点：`handle-event(ev: types::Event)`（旧字符串双参已废）与 `handle-service(service, method, payload)`；出站效果走 `effect-http-request`（旧 `effect-http-get` 已废）。
- 测试外置（`tests/builders.rs`，native 单元级）；guest 全链路由 `tools/wasm-fixtures` 真实组件与 server `wasm` 测试覆盖。

## 验证与契约同步

```bash
~/.cargo/bin/cargo test --manifest-path server/crates/xiaozhi-plugin-sdk/Cargo.toml
./tools/wasm-fixtures/build.sh          # 改契约后必须重建全部夹具
./tools/build-system-plugins.sh         # 改契约后必须重建全部 15 个系统插件产物
```

⛔ **改 `server/wit/plugin.wit` 的完整纪律**：重编本 crate → 重建夹具与 `plugins-src/*` 全部 guest → 重建 `build/system-plugins/*.wasm` → 远端重新 `upload/raw → register → activate`。宿主与 guest 不同版本时 activate 直接失败；同版本号重传会被 409「版本不升级」拒绝，必要时同步升版本号。
