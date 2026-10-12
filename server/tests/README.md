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
