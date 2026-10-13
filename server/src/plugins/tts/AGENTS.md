# TTS 实现维护

[trait 与配置](<mod.rs>)产流式增量 PCM，下游负责编码/发送；本地[Kokoro](<kokoro.rs>)与在线[AIUI TTS](<aiui.rs>)共享这条契约。

- 讯飞 xfyun 使用 AIUI 主动合成链路；[xfyun helper](<xfyun.rs>)提供凭据/签名，不把旧 v2 TTS URL 当当前音色契约。
- 回调 false 表示取消；下载背压等待不计入上游活跃超时，ActiveDeadline 只算上游不给数据的时间。不得改成简单墙钟总超时。
- 流式音频分片交给共同下行流水线，重采样与 Opus 编码器跨片复用；不要每片重建或整段收完才播放。
- 测试台临时 vcn 只影响本次合成；失败返回结构化 tts_test error，不能静默无声。
- 新供应商加实现、参数结构、engine 声明、注册表描述符/字段；客户端引擎清单和字段区尚非全自动 schema，需同步维护。
- 密钥遵守[配置端点维护](<../../app/ws/config/AGENTS.md>)；真实音色目录是低并发 API 探测，不是静态候选列表即有效列表。

回归：[TTS 测试](<tests.rs>)。下行维护见[下行指引](<../../app/downlink/AGENTS.md>)，音色探测见[音色说明](<../../app/voices/AGENTS.md>)。
