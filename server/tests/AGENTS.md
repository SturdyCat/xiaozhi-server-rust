# tests/ 联调说明

## mock_client.py —— 协议联调客户端（零第三方依赖，纯标准库）

模拟 macApp 测试台（hello 带 `test:true`），覆盖：

1. 握手/协商（服务器 hello + audio_params）
2. `asr_test` start/stop → `stt` 回包
3. `tts_test` → `tts start / sentence_start / 二进制 Opus 帧 / tts stop`
4. `llm_test` → 回包到达（state=ok|error 均算端点可达）
5. **abort 场景（fatal）**：发 `abort` 帧后用 `asr_test` 存活探针验证连接可用
6. **wakeup 场景（容忍型）**：`listen detect` → 收到 tts 序列即通过；
   未收到（[wakeup] 未启用 / TTS 不可用）只记 WARN，不算失败

任一 fatal 失败 → 退出码 1（可作 CI 门槛）。

## 本地运行

```bash
# 1) 起服务端（带真实引擎需 sherpa + 模型；或用 docker-compose 自动下载模型）
cd server && cargo run --features sherpa -- --config config.toml

# 2) 跑协议联调（若 [server].expected_token 非空需 --token）
python3 tests/mock_client.py --host 127.0.0.1 --port 8000 [--token <bearer>]
```

## 单元测试

```bash
cd server
cargo test                        # default features（无 sherpa，全部单测）
cargo test --features sherpa      # 真实引擎路径单测（需 libopus / 模型）
```

> 本机 cargo 不在 PATH 时：`export PATH="$HOME/.cargo/bin:$PATH"`；
> 镜像 404 时追加 `--config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'`。


---

## 服务端测试与回归

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

### 8. 测试与回归

#### 单元自测（Rust）

协议/音频/配置/插件/记忆/overlay/preset 各层均含 `#[cfg(test)]` 用例（二进制版本往返、密钥护栏与部分更新语义、overlay 白名单拒止、预设落盘往返、registry 护栏、VAD/ASR 桩等，全量 468+ 例）：

```bash
cd server && ~/.cargo/bin/cargo test
```

#### 协议联调（Python 测试台客户端）

`tests/mock_client.py` 是**零第三方依赖**的 WebSocket 客户端（纯标准库实现 RFC 6455 帧），场景化回归：握手协商、`asr_test`/`tts_test`/`llm_test`、`abort` 打断后连接存活探针（fatal）、唤醒问候（容忍型）、多轮对话：

```bash
cd server && cargo run --release --features sherpa -- --config config.toml &
python3 server/tests/mock_client.py        # 逐场景 PASS/WARNING，末尾汇总
```

可用参数：`--host 127.0.0.1 --port 8000 --token <bearer>`（`expected_token` 非空时传 `--token`）。

---

## 上级入口

[上级 AGENTS](<../AGENTS.md>)
