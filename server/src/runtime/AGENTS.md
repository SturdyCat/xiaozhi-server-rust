# 引擎租约与热刷新

继承[服务端指引](<../../AGENTS.md>)。进程装配入口为[runtime 模块](<mod.rs>)（原 `runtime/mod.rs`，已重命名为 `runtime/mod.rs`）；[刷新](<refresh.rs>)负责配置来源与非 TTS 重建，[TTS](<tts.rs>)负责原子租约和语言池。

- `TtsLease` 同时持有实际实例、实际构建配置、注册表 fingerprint 和 generation；不实现 Debug，签名与配置可能含凭据，禁止打印。
- `Engines::tts_lease()` 是生产会话入口；会话固定持有租约实现 NextSession。设备 overlay 仅用 `with_synthesis_params` 修改 speaker/speed，不可把未构建的 model/voice/凭据写进实际身份。
- `refresh_lock` 必须覆盖**读配置之前**到发布完成，串行化测试台与会话刷新；`TtsSlot` 内构建也串行。失败保持整份旧租约，不允许先替实例后补签名，不允许旧读盘快照在新刷新后发布。
- generation 表示实际实例构建代际；只改调用参数不重建。语言池按 generation/lang 预留 single-flight cell，同语言并发只建一次、不同语言独立建模；失败 flight 共享默认实际租约并允许后续请求重试。发布新代与旧池失效在同一锁事务内完成，过期语言构建可供旧调用使用，但不能回灌新代缓存。
- `HotAudioKey::from_lease` 统一欢迎语、问候、固定应答与普通句子身份。旧会话/旧身份仍有效，无须热切换时全清内存缓存；新 fingerprint 自然隔离。Kokoro 注册表签名覆盖全部构造输入。
- 外置测试 [tts/tests.rs](<tts/tests.rs>) 使用 fake 引擎，覆盖失败身份保留、成功/旧会话固定、并发刷新、全构建签名和磁盘缓存；测试不证明真实模型可用。

返回[服务端指引](<../../AGENTS.md>)。
