# WASM 测试夹具

为 `plugins/wasm/tests.rs` 提供真实 guest 组件，契约 `xiaozhi:plugin@0.2.0`。**产物不入库**（git 忽略）：测试经 `ensure_fixture` 在缺失/源更新时本地重建。当前平台权威说明见 [plugins/wasm/AGENTS.md](<../../plugins/wasm/AGENTS.md>)。

## 构建

```bash
./build.sh   # 前置：rustup target add wasm32-unknown-unknown；brew install wasm-tools
```

- crate：`hello`（官方默认：全链路 + kv 读写闭环 + S3l `tcp-connect-probe / random-probe（S3s 宿主随机源：长度 + 两次互异 + 越界拒绝）` 自检心证）、`greeter-user`（用户自建：同扩展点替换演练 +
  无权限对照面）、`trap`（作恶：无限循环/无限分配）、`tuyasign`（**协议语义单源对拍**：`#[path]`
  引入 `server/src/device/tuya/protocol.rs`，guest 内执行官方 v1.0 签名，由
  `plugins/wasm/tests/protocol.rs` 与宿主黄金向量逐字节对拍——阶段三 S3b 证据）、
  `guest-alloc`（共用分配器：dlmalloc 可回收实现，S3n 起；此前 bump 只增不回收属长跑 OOM 隐患）。
- 三个夹具（hello / greeter-user / trap）与 `guest-alloc` 共用分配器**已全部迁到 0.2**：`host_core as host`、类型化 `handle_event(ev)`、`EffectHttpGet` → `EffectHttpRequest`，并补齐 `handle_service` 桩。
- hello/greeter-user 经 **guest SDK**（`server/crates/xiaozhi-plugin-sdk`）取契约绑定与构建器；
  trap 刻意走裸 `wit_bindgen::generate!`——证明契约不绑死 SDK（等价公民的另一形态）。
- 构建：`cargo build --release --target wasm32-unknown-unknown` + `wasm-tools component new`。
- 契约源：`server/wit/plugin.wit`（`package xiaozhi:plugin@0.2.0`，与宿主 bindgen 同一份）——**改契约必须重建全部夹具**，否则宿主侧 world 校验会以组件不匹配失败；同时还要重建 `plugins-src/*` 与 `build/system-plugins/*.wasm`，并重新上传远端（见 [plugins/wasm/AGENTS.md](<../../plugins/wasm/AGENTS.md>)）。

## 已知坑（实测记录，勿再踩）

- **guest 分配器必须从 `__heap_base` 起分配**：线性内存低地址是静态数据段（字符串字面量/
  静态量/shadow stack），从 0/1 起 bump 第一笔分配就破坏自身 → guest panic 陷阱。
- **GlobalAlloc 不可返回 0**：Rust 语义中 null 表示分配失败，bump 起点要 max(1)。
- wit-bindgen guest 导出 trait 路径是 `crate::exports::lifecycle::Guest`（内联 world 接口
  不带包名空间），与直觉的 `exports::xiaozhi::plugin::...` 不同；0.2 把 `import host` 拆成四个
  Host trait 后**导出路径不变**，只多了 `handle_service` 实现点与四个导入面的 trait 实现。
- **共享绑定 crate（SDK）的两个必开选项**：`pub_export_macro: true`（export! 宏 pub 化）
  **和** `default_bindings_module: "xiaozhi_plugin_sdk"`（宏展开体路径默认 `self` =
  调用方 crate 根，guest 那没有绑定模块）；只开前者报 E0433 `could not find export`。
  手工 `pub use export;` 不可行（E0364/E0252）。
- **夹具产物名连字符转下划线**：crate `greeter-user` 的 cdylib 产物是
  `greeter_user.wasm`（build.sh 已按此映射）。
- 路径依赖层级：本目录 crate 在 `tools/wasm-fixtures/<crate>/`，指向仓库内其他位置需
  三级向上（`../../../server/...`）。
