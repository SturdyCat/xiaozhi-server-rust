# 音频媒体指引

继承[服务端指引](<../../AGENTS.md>)。音频处理：Opus 编解码、流式重采样、PCM 换算、下行预算、热欢迎语音频库、类型化媒体契约。

- Opus 编解码 `opus.rs`：上行设备 Opus → f32 PCM（喂 VAD/ASR），下行 TTS PCM → 按下行采样率编码。**下行编码必须用 `OpusFrameEncoder`（流式、有状态），不要每帧新建编码器**。真实路径用 `audiopus`，需系统 libopus（`sherpa` feature）；未启用 feature 时提供占位实现仅供编译 / 测试，生产不含。
- 流式重采样 `resample.rs`：逐分片推送、跨分片保持相位连续；分片边界各自独立重采样会产生样本级跳变 / 爆音。
- PCM 换算 `pcm.rs`：纯函数，唯一调用方是 AIUI 全链路（只收 16k i16 PCM）。
- 下行预算 `budget.rs`：非阻塞的时长 / 字节双重预留；音频过载永不阻塞控制面。
- 热欢迎语 `hot_audio.rs`：所有生产 TTS 文本共用的有界持久化音频库；磁盘存裸 PCM（24kHz mono S16LE），文件名 = 合成身份 FNV-1a；欢迎语 / 问候 / 固定应答 / 普通句子身份统一走 `HotAudioKey::from_lease`，Kokoro 签名覆盖全部构造输入。
- 媒体契约 `media.rs`：类型化端到端媒体契约；wire 成帧属传输适配层职责。

## 上级入口

[服务端指引](<../../AGENTS.md>)

## 直属下级

- Opus `opus.rs`
- 重采样 `resample.rs`
- PCM `pcm.rs`
- 预算 `budget.rs`
- 热音频 `hot_audio.rs`
- 媒体契约 `media.rs`
