#import "XiaoZhiModule.h"
#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <QuartzCore/QuartzCore.h> // CACurrentMediaTime（试听进度推算）
#import <UIKit/UIKit.h>           // UIPasteboard（复制到系统剪贴板）

// ⚠️ 参数/回调 key 不能自定义：KR_PARAM_KEY/KR_CALLBACK_KEY 是 OpenKuiklyIOSRender
//    KRBaseModule.h 声明的 extern 常量（实际值为 @"param"/@"callback"）。
//    此前用 #ifndef 兜底定义 @"__kr_param__"，运行期取不到参数（params 恒为空字典），
//    WS 连接实际从未拿到 url（会以 invalid url 失败或此处崩溃）。

static const uint32_t kUplinkSampleRate = 16000;   // 上行（麦克风）采样率，与 server 协商一致
static const uint32_t kDownlinkSampleRate = 24000; // 下行（TTS）默认采样率，实际以 server hello 为准
static const uint32_t kFrameDurationMs = 60;       // 上下行帧长，与 server 一致

// ===== 新版 API 兼容封装（Xcode 27 SDK 把 connect/play/installTap 改为带 error 的版本）=====
// 部署目标 macOS 12（Catalyst iOS 15），新 API 需 Catalyst 27+：可用则用新 API（并回报错误），
// 否则回退旧 API（仅弃用、功能一致，局部抑制弃用告警保持构建零警告）。
static BOOL XZEngineConnect(AVAudioEngine *engine, AVAudioNode *src, AVAudioNode *dst, AVAudioFormat *fmt) {
    NSError *err = nil;
    BOOL ok;
    if (@available(macCatalyst 27.0, *)) {
        ok = [engine connect:src to:dst format:fmt error:&err];
    } else {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        [engine connect:src to:dst format:fmt];
        ok = YES;
#pragma clang diagnostic pop
    }
    if (!ok || err) NSLog(@"[XiaoZhi] engine connect 失败: %@", err);
    return ok;
}

static void XZPlayerPlay(AVAudioPlayerNode *player) {
    NSError *err = nil;
    if (@available(macCatalyst 27.0, *)) {
        if (![player playAndReturnError:&err] || err) NSLog(@"[XiaoZhi] player play 失败: %@", err);
    } else {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        [player play];
#pragma clang diagnostic pop
    }
}

@interface XiaoZhiModule ()
@property (nonatomic, strong, nullable) NSURLSessionWebSocketTask *webSocket;
@property (nonatomic, strong, nullable) NSURLSession *session;
@property (nonatomic, strong) AVAudioEngine *audioEngine;
@property (nonatomic, strong) AVAudioPlayerNode *playerNode;
@property (nonatomic, strong, nullable) AVAudioInputNode *micNode;
@property (nonatomic, assign) BOOL recording;
@property (nonatomic, copy, nullable) id asrCallback;   // keepCallbackAlive：待服务端回 stt 时回调
@property (nonatomic, copy, nullable) id speakCallback; // 待 tts stop（或 tts_test 错误帧）时回调
@property (nonatomic, copy, nullable) NSString *speakEngine;  // 本次合成引擎名（tts_test 成功帧携带）
@property (nonatomic, copy, nullable) id llmCallback;   // 待服务端回 llm_test 时回调
@property (nonatomic, assign) uint32_t downlinkSampleRate;
// ===== Opus 编解码（系统 AudioToolbox，实测可用；Catalyst 下无需第三方库）=====
@property (nonatomic, assign) AudioConverterRef downlinkDecoder; // Opus(下行率) → LPCM float32
@property (nonatomic, assign) AudioConverterRef uplinkEncoder;   // LPCM float32(48k) → Opus(16k)
@property (nonatomic, strong) NSMutableData *micAccum;           // 麦克风 48k 采样累积（攒满一帧再编码）
@property (nonatomic, assign) UInt32 micFrameSamples;            // 每帧 48k 样本数（2880 = 60ms）
// ===== 试听/波形（录音与 TTS 统一存 24k float32 单声道，player 连接格式即 24k/1ch）=====
@property (nonatomic, strong) NSMutableData *recordingPcm;       // 本次录音缓冲（试听 + 波形）
@property (nonatomic, strong) NSMutableData *ttsPcm;             // 最近一次 TTS 合成缓冲（试听 + 波形）
@property (nonatomic, assign) BOOL playbackActive;               // 正在试听（区别于下行实时播放）
@property (nonatomic, assign) CFTimeInterval playbackStartedAt;  // 试听起始时钟（进度 = now - startedAt）
@property (nonatomic, assign) NSUInteger recSamples24k;          // 录音中已采集的 24k 样本数（只增计数，避免跨线程读缓冲）
@property (nonatomic, assign) CFTimeInterval asrSentAt;          // sendAsr 发出时刻（统计识别往返耗时）
@property (nonatomic, assign) NSUInteger decodeFailCount;        // 下行解码连续失败计数（日志节流）
@property (nonatomic, assign) BOOL loggedFirstDecode;            // 首个下行包解码成功仅打一条日志
@property (nonatomic, assign) NSUInteger ttsDecodedPackets;      // 本次合成成功解码的包数（tts stop 汇总）
@property (nonatomic, assign) NSUInteger ttsFailCount;           // 本次合成解码失败的包数（tts stop 汇总）
@end

@implementation XiaoZhiModule

+ (NSString *)moduleName { return @"XiaoZhiModule"; }

#pragma mark - 参数解析

// Kotlin 侧 toNative 的 param 类型不定：传 JSON 字符串 → 原生收 NSString；
// 直接传 JSONObject 对象 → 桥接为 SharedCoreJSONObject（不能下标取值，否则
// unrecognized selector objectForKeyedSubscript: 崩溃）。统一解析为 NSDictionary。
- (NSDictionary *)parseParams:(id)raw {
    if ([raw isKindOfClass:[NSDictionary class]]) return raw;
    if ([raw isKindOfClass:[NSString class]]) return [raw hr_stringToDictionary] ?: @{};
    if (raw) {
        // SharedCoreJSONObject 的 description 即其 Kotlin toString() = JSON 文本
        id d = [[raw description] hr_stringToDictionary];
        if ([d isKindOfClass:[NSDictionary class]]) return d;
    }
    return @{};
}

#pragma mark - Module 方法（方法名与 Kotlin 侧 toNative(methodName=...) 一一对应）

// connect(url, token) → 建立 WS 并发送 hello
- (void)connect:(NSDictionary *)args {
    NSDictionary *params = [self parseParams:args[KR_PARAM_KEY]];
    id callback = args[KR_CALLBACK_KEY];
    NSString *url = params[@"url"] ?: @"";
    NSString *token = params[@"token"] ?: @"";
    NSLog(@"[XiaoZhi] connect url=%@ token=%@", url, token);

    [self setupAudio];

    NSURLComponents *c = [NSURLComponents componentsWithString:url];
    if (token.length > 0) {
        NSMutableArray *qry = [NSMutableArray arrayWithArray:c.queryItems ?: @[]];
        [qry addObject:[NSURLQueryItem queryItemWithName:@"token" value:token]];
        c.queryItems = qry;
    }
    NSURL *wsUrl = c.URL;
    if (!wsUrl) {
        [self invoke:callback result:@{@"success": @(NO), @"error": @"invalid url"} success:NO error:@"invalid url"];
        return;
    }

    self.session = [NSURLSession sessionWithConfiguration:[NSURLSessionConfiguration defaultSessionConfiguration]];
    self.webSocket = [self.session webSocketTaskWithURL:wsUrl];
    [self.webSocket resume];
    [self receiveLoop];

    // hello：version=1 → 上行裸 Opus（v1）；audio_params 声明本端 16k/opus。
    // test=YES：标记本连接为管理端测试台（服务端据此受理 asr_test/tts_test/llm_test
    // 三种独立服务请求；ESP 设备不带此参数，走正式流水线，测试端点被忽略）。
    [self sendJSON:@{
        @"type": @"hello",
        @"version": @(1),
        @"test": @(YES),
        @"audio_params": @{
            @"format": @"opus",
            @"sample_rate": @(kUplinkSampleRate),
            @"channels": @(1),
            @"frame_duration": @(kFrameDurationMs)
        }
    }];

    [self invoke:callback result:@{@"success": @(YES)} success:YES error:nil];
}

- (void)disconnect:(NSDictionary *)args {
    [self stopMic];
    // 真实 API：cancelWithCloseCode:reason:（关闭码枚举是 NormalClosure，无 "NormalProtocolStatus"）
    [self.webSocket cancelWithCloseCode:NSURLSessionWebSocketCloseCodeNormalClosure reason:nil];
    self.webSocket = nil;
    id callback = args[KR_CALLBACK_KEY];
    if (callback) [self invoke:callback result:@{@"success": @(YES)} success:YES error:nil];
}

// startAsr() → 申请麦克风权限并开始采集，发送 asr_test start；服务端缓冲整段音频（跳过 VAD）。
// 本地同时累积 24k PCM（recordingPcm）供停止后试听/波形；「发送识别」由 sendAsr 单独触发。
// 回调时序（asrCallback keepCallbackAlive，多次回调）：
//   ① 录音真正开始 → {started:true}（UI 据此进入录音态并开始计秒，不再靠猜测延时）
//   ② 启动失败（权限被拒/设备异常）→ {error:"..."}
//   ③ 识别结果 → {text:"...", elapsedMs:N}（sendAsr → stt 的服务端往返耗时）
- (void)startAsr:(NSDictionary *)args {
    self.asrCallback = args[KR_CALLBACK_KEY]; // 保留，待 started/stt 时回调
    self.recordingPcm = [NSMutableData data];
    self.recSamples24k = 0;
    __weak typeof(self) weak = self;
    [self requestMicPermission:^(BOOL ok) {
        typeof(self) strong = weak;
        if (!strong) return;
        if (!ok) {
            [strong invoke:strong.asrCallback result:@{@"error": @"麦克风权限被拒绝（系统设置 → 隐私与安全性 → 麦克风）"} success:NO error:@"mic denied"];
            strong.asrCallback = nil;
            return;
        }
        strong.recording = YES;
        [strong sendJSON:@{@"type": @"asr_test", @"action": @"start"}];
        if ([strong startMic]) {
            // 实际开始录音后才回报（installTap + 引擎启动均成功）
            [strong invoke:strong.asrCallback result:@{@"started": @(YES)} success:YES error:nil];
        } else {
            strong.recording = NO;
            [strong invoke:strong.asrCallback result:@{@"error": @"录音设备启动失败（检查输入设备）"} success:NO error:@"mic start failed"];
            strong.asrCallback = nil;
        }
    }];
}

// stopRecording() → 仅停止采集（保留录音缓冲供试听），不发 asr_test stop。
- (void)stopRecording:(NSDictionary *)args {
    self.recording = NO;
    [self stopMic];
    id callback = args[KR_CALLBACK_KEY];
    double seconds = self.recordingPcm.length / (24000.0 * sizeof(float));
    [self invoke:callback result:@{@"seconds": @(seconds)} success:YES error:nil];
}

// sendAsr() → 发送 asr_test stop；服务端对整段（此前流式上传的）做一次性识别并回 stt
// （stt 经既有 asrCallback 通道回到 Kotlin）。发出时刻记入 asrSentAt，供统计识别耗时。
- (void)sendAsr:(NSDictionary *)args {
    self.asrSentAt = CACurrentMediaTime();
    [self sendJSON:@{@"type": @"asr_test", @"action": @"stop"}];
}

// speak(text, speaker, lang, speed) → 发送 tts_test；服务端合成并流式下发 Opus 音频。
// 下行 PCM 同时累积到 ttsPcm，tts stop 后可供试听/波形（playTts）。
- (void)speak:(NSDictionary *)args {
    NSDictionary *params = [self parseParams:args[KR_PARAM_KEY]];
    self.speakCallback = args[KR_CALLBACK_KEY];
    self.speakEngine = nil;
    self.ttsPcm = [NSMutableData data];
    self.ttsDecodedPackets = 0;
    self.ttsFailCount = 0;
    self.decodeFailCount = 0;
    // 每次新合成开始时重置一次解码器状态：与编码端「每句一个新编码器」的状态边界对齐
    // （句内全程连续解码、帧间状态保留；句间边界各重置一次，避免上句残留状态污染新句开头）。
    // ⚠️ Reset 不会清除 primeInfo 属性（转换器配置与运行状态分离），粘滞防护仍有效。
    if (self.downlinkDecoder) AudioConverterReset(self.downlinkDecoder);
    // ⚠️ 不在这里启动音频引擎：实机对比发现「点击即启动引擎」会在麦克风未授权时
    //    触发 AUIOBase/-50 刷屏；改为首帧到达（playPcm→ensureAudioEngine）时才启动，
    //    与实测零噪音路径一致（synthesize 需 1s+，引擎启动 ~百 ms，无听感影响）。
    // 新一次合成前丢弃上一段仍在排队的音频（否则旧尾音与新音频叠播）
    [self.playerNode stop];
    [self.playerNode reset];
    XZPlayerPlay(self.playerNode);
    [self sendJSON:@{
        @"type": @"tts_test",
        @"text": params[@"text"] ?: @"",
        @"speaker": params[@"speaker"] ?: @(0),
        @"lang": params[@"lang"] ?: @"zh",
        @"speed": params[@"speed"] ?: @(1.0)
    }];
}

// 备注：speak(...) 发送的即 tts_test；服务端先回一帧 tts_test 结果（含 engine 与错误原因），
// 再照常下发音频并以 tts stop 收尾。handleText 的 tts_test 分支：error 就地消费回调、
// ok 仅记 engine 名，tts stop 时统一回报 {state:"stop"|"error", engine, text}。

// llmTest(text) → 发送 llm_test；服务端按磁盘上最新 [llm] 配置直调 LLM（单轮无历史），
// 结果经 llm_test 回调返回：{state: "ok"|"error", text: 回复正文或错误信息, elapsedMs}。
- (void)llmTest:(NSDictionary *)args {
    NSDictionary *params = [self parseParams:args[KR_PARAM_KEY]];
    self.llmCallback = args[KR_CALLBACK_KEY];
    [self sendJSON:@{
        @"type": @"llm_test",
        @"text": params[@"text"] ?: @""
    }];
}

#pragma mark - 系统剪贴板

// copyText(text) → 写入系统剪贴板（管理端复制识别结果/LLM 回复用）。
// Catalyst 下 NSPasteboard/UIPasteboard 由渲染层 KRUIKit 兼容宏统一，这里直接用 UIPasteboard。
- (void)copyText:(NSDictionary *)args {
    NSDictionary *params = [self parseParams:args[KR_PARAM_KEY]];
    NSString *text = params[@"text"] ?: @"";
    [UIPasteboard generalPasteboard].string = text;
}

#pragma mark - 试听 / 波形（录音与 TTS 共用，缓冲统一 24k float32 单声道）

// playRecording() / playTts() → 从头整段播放对应缓冲（playSource: 共用实现）
- (void)playRecording:(NSDictionary *)args {
    [self playSource:self.recordingPcm callback:args[KR_CALLBACK_KEY]];
}

- (void)playTts:(NSDictionary *)args {
    [self playSource:self.ttsPcm callback:args[KR_CALLBACK_KEY]];
}

- (void)playSource:(NSData *)pcm callback:(id)callback {
    if (pcm.length < sizeof(float)) {
        [self invoke:callback result:@{@"duration": @(0)} success:YES error:nil];
        return;
    }
    [self ensureAudioEngine]; // 懒启动：播放前才拉起引擎
    [self.playerNode stop];
    [self.playerNode reset];
    uint32_t sr = self.downlinkSampleRate ?: kDownlinkSampleRate;
    AVAudioFormat *fmt = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                          sampleRate:sr channels:1 interleaved:NO];
    AVAudioFrameCount frames = (AVAudioFrameCount)(pcm.length / sizeof(float));
    AVAudioPCMBuffer *buf = [[AVAudioPCMBuffer alloc] initWithPCMFormat:fmt frameCapacity:frames];
    buf.frameLength = frames;
    memcpy(buf.floatChannelData[0], pcm.bytes, pcm.length);
    self.playbackActive = YES;
    self.playbackStartedAt = CACurrentMediaTime();
    __weak typeof(self) weak = self;
    [self.playerNode scheduleBuffer:buf completionHandler:^{
        // 数据消费完毕（播完或被 stop 打断）；派主线程与轮询侧一致。
        // 播完状态由 Kotlin 轮询 getAudioState 发现（playing=false）。
        dispatch_async(dispatch_get_main_queue(), ^{
            typeof(self) strong = weak;
            if (!strong) return;
            strong.playbackActive = NO;
        });
    }];
    XZPlayerPlay(self.playerNode);
    double duration = pcm.length / (sr * sizeof(float));
    [self invoke:callback result:@{@"duration": @(duration)} success:YES error:nil];
}

// stopPlayback() → 停止试听
- (void)stopPlayback:(NSDictionary *)args {
    [self.playerNode stop];
    [self.playerNode reset];
    XZPlayerPlay(self.playerNode);
    self.playbackActive = NO;
}

// getAudioState(source) → {wave:[64 桶峰值 0~1], duration, playing, position, recordingSeconds, recording}
// Kotlin 侧 100ms 轮询驱动播放进度/录音计秒；source: "recording" | "tts"
- (void)getAudioState:(NSDictionary *)args {
    NSDictionary *params = [self parseParams:args[KR_PARAM_KEY]];
    id callback = args[KR_CALLBACK_KEY];
    NSData *pcm = [params[@"source"] isEqualToString:@"tts"] ? self.ttsPcm : self.recordingPcm;
    uint32_t sr = self.downlinkSampleRate ?: kDownlinkSampleRate;
    double duration = pcm.length / (sr * sizeof(float));
    // 播放进度用「起始时钟 + 播放速率 1」推算（player timeline 在 stop 后归零，时钟更稳）；
    // 是否仍在播以 AVAudioPlayerNode.isPlaying 为准（schedule 队列播完自动转 NO）。
    BOOL playing = self.playbackActive && self.playerNode.isPlaying;
    double position = playing ? CACurrentMediaTime() - self.playbackStartedAt : 0;
    if (position > duration) position = duration;
    NSArray *wave = [self waveformOf:pcm buckets:64];
    // 录音计秒：录音中按已采集样本数回报实时秒数（24k 单声道）
    double recordingSeconds = self.recSamples24k / 24000.0;
    [self invoke:callback result:@{
        @"wave": wave ?: @[],
        @"duration": @(duration),
        @"playing": @(playing),
        @"position": @(position),
        @"recordingSeconds": @(recordingSeconds),
        @"recording": @(self.recording),
    } success:YES error:nil];
}

// 波形包络：等分为 buckets 段，每段取峰值绝对值并按全局峰值归一化到 [0,1]
- (NSArray *)waveformOf:(NSData *)pcm buckets:(NSInteger)buckets {
    NSInteger frames = pcm.length / sizeof(float);
    if (frames <= 0 || buckets <= 0) return @[];
    const float *s = pcm.bytes;
    NSMutableArray *out = [NSMutableArray arrayWithCapacity:buckets];
    double peak = 1e-6;
    for (NSInteger b = 0; b < buckets; b++) {
        NSInteger lo = b * frames / buckets;
        NSInteger hi = MAX(lo + 1, (b + 1) * frames / buckets);
        double m = 0;
        for (NSInteger i = lo; i < hi && i < frames; i++) {
            double a = fabs((double)s[i]);
            if (a > m) m = a;
        }
        if (m > peak) peak = m;
        [out addObject:@(m)];
    }
    NSMutableArray *norm = [NSMutableArray arrayWithCapacity:buckets];
    for (NSNumber *v in out) {
        double x = v.doubleValue / peak;
        [norm addObject:@(0.12 + 0.88 * x)]; // 底噪高度 12%，避免静音段全空
    }
    return norm;
}

#pragma mark - WebSocket

- (void)sendJSON:(NSDictionary *)dict {
    if (!self.webSocket) return;
    NSError *err;
    NSData *data = [NSJSONSerialization dataWithJSONObject:dict options:0 error:&err];
    if (err) { NSLog(@"[XiaoZhi] json error: %@", err); return; }
    NSString *str = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    // 真实 API：sendMessage:completionHandler:（无 sendString:）
    [self.webSocket sendMessage:[[NSURLSessionWebSocketMessage alloc] initWithString:str]
              completionHandler:^(NSError * _Nullable e) {
        if (e) NSLog(@"[XiaoZhi] send error: %@", e);
    }];
}

- (void)receiveLoop {
    __weak typeof(self) weak = self;
    // 真实 API：receiveMessageWithCompletionHandler:（无 receiveWithCompletionHandler:）
    [self.webSocket receiveMessageWithCompletionHandler:^(NSURLSessionWebSocketMessage * _Nullable msg, NSError * _Nullable error) {
        typeof(self) strong = weak;
        if (!strong || error) return;
        // playerNode/audioEngine 的操作统一派发主线程（AVAudioEngine 的属性/调度需同一线程同步，
        // URLSession 回调默认在非主线程，直接操作有竞态风险）。
        if (msg.type == NSURLSessionWebSocketMessageTypeString) {
            NSString *s = msg.string;
            dispatch_async(dispatch_get_main_queue(), ^{ [strong handleText:s]; });
        } else if (msg.type == NSURLSessionWebSocketMessageTypeData) {
            NSData *d = msg.data;
            dispatch_async(dispatch_get_main_queue(), ^{ [strong handleBinary:d]; });
        }
        [strong receiveLoop];
    }];
}

- (void)handleText:(NSString *)text {
    NSData *d = [text dataUsingEncoding:NSUTF8StringEncoding];
    NSDictionary *dict = [NSJSONSerialization JSONObjectWithData:d options:0 error:nil];
    if (!dict) return;
    NSString *type = dict[@"type"];
    if ([type isEqualToString:@"hello"]) {
        NSDictionary *ap = dict[@"audio_params"];
        if (ap[@"sample_rate"]) {
            uint32_t sr = [ap[@"sample_rate"] unsignedIntValue];
            if (sr != self.downlinkSampleRate) {
                self.downlinkSampleRate = sr;
                [self rebuildDownlinkDecoder];              // 下行采样率以 server hello 为准
                [self reconnectPlayerForSampleRate:sr];     // player 连接格式联动（否则变速/异常）
            }
        }
        return;
    }
    if ([type isEqualToString:@"stt"]) {
        NSString *t = dict[@"text"] ?: @"";
        if (self.asrCallback) {
            // 识别耗时：sendAsr（发出 asr_test stop）→ 收到本条 stt 的往返ms
            double elapsedMs = self.asrSentAt > 0 ? (CACurrentMediaTime() - self.asrSentAt) * 1000.0 : 0;
            self.asrSentAt = 0;
            [self invoke:self.asrCallback result:@{@"text": t, @"elapsedMs": @(elapsedMs)} success:YES error:nil];
            self.asrCallback = nil; // one-shot
        }
        return;
    }
    if ([type isEqualToString:@"tts"]) {
        NSString *state = dict[@"state"];
        if ([state isEqualToString:@"stop"]) {
            // 每次合成收尾无条件汇总：实机日志可直接确认「多少包解码成功 / 多少帧 / 失败几包」
            NSUInteger frames = self.ttsPcm.length / sizeof(float);
            uint32_t sr = self.downlinkSampleRate ?: kDownlinkSampleRate;
            NSLog(@"[XiaoZhi] TTS 下行统计：解码 %lu 包 / %lu 帧（%.2fs），失败 %lu 包",
                  (unsigned long)self.ttsDecodedPackets, (unsigned long)frames,
                  frames / (double)sr, (unsigned long)self.ttsFailCount);
            if (self.speakCallback) {
                [self invoke:self.speakCallback result:@{
                    @"state": @"stop",
                    @"engine": self.speakEngine ?: @""
                } success:YES error:nil];
                self.speakCallback = nil;
            }
        }
        return;
    }
    if ([type isEqualToString:@"llm_test"]) {
        if (self.llmCallback) {
            [self invoke:self.llmCallback result:@{
                @"state": dict[@"state"] ?: @"error",
                @"text": dict[@"text"] ?: @"",
                @"elapsedMs": dict[@"elapsed_ms"] ?: @(0)
            } success:YES error:nil];
            self.llmCallback = nil; // one-shot
        }
        return;
    }
    if ([type isEqualToString:@"tts_test"]) {
        // 服务端合成结果帧（音频下发前到达，含引擎名与错误原因）：
        // - state=error：不会再有 tts stop，就地消费回调（one-shot）；
        // - state=ok：仅记下引擎名，等 tts stop 统一回报（音频随后照常下发）。
        NSString *st = dict[@"state"] ?: @"error";
        self.speakEngine = dict[@"engine"] ?: @"";
        if ([st isEqualToString:@"error"] && self.speakCallback) {
            [self invoke:self.speakCallback result:@{
                @"state": @"error",
                @"engine": self.speakEngine ?: @"",
                @"text": dict[@"text"] ?: @""
            } success:YES error:nil];
            self.speakCallback = nil;
        }
        return;
    }
}

- (void)handleBinary:(NSData *)opus {
    if (opus.length == 0) return;
    NSData *pcm = [self decodeOpus:opus];
    if (!pcm.length) return;
    // 累积到 TTS 缓冲（24k float32，与下行采样率一致），tts stop 后供试听/波形。
    // 懒初始化：正常流程 speak 已建好缓冲；但下行若未经 speak 到达（诊断/mock 场景），
    // 对 nil 调 appendData 是静默空操作，会丢缓冲——这里兜底。
    if (!self.ttsPcm) self.ttsPcm = [NSMutableData data];
    [self.ttsPcm appendData:pcm];
    [self playPcm:pcm];
}

#pragma mark - 音频采集 / 播放

- (void)setupAudio {
    if (self.audioEngine) return;
    self.downlinkSampleRate = kDownlinkSampleRate; // 默认 24k；hello 一致则不触发无谓重连
    self.audioEngine = [[AVAudioEngine alloc] init];
    self.playerNode = [[AVAudioPlayerNode alloc] init];
    [self.audioEngine attachNode:self.playerNode];
    // ⚠️ player→mainMixer 必须**显式指定**与调度 buffer 相同的声道数（下行单声道）。
    //    实测（本机 AVAudioEngine）：format:nil 时 mixer 协商为 48000Hz/2ch，调度
    //    1ch buffer 直接抛 "required condition is false: _outputFormat.channelCount ==
    //    buffer.format.channelCount"。采样率也会按连接解释（不抛异常但变速），
    //    故连接采样率与下行采样率必须一致（hello 后如变化走 reconnectPlayerForSampleRate:）。
    AVAudioFormat *playerFormat = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                                   sampleRate:self.downlinkSampleRate
                                                                     channels:1
                                                                  interleaved:NO];
    XZEngineConnect(self.audioEngine, self.playerNode, self.audioEngine.mainMixerNode, playerFormat);
    // ⚠️ 这里**只建图不启动引擎**（懒启动，见 ensureAudioEngine）：
    //    ① 引擎一启动就会拉起 full-duplex IO，麦克风未授权时 input 侧 0 声道 →
    //       AUIOBase "no channels in channel map" + AudioConverter -50 刷屏（connect 即出现）；
    //    ② connect 阶段没有任何音频要播，启动引擎纯属浪费。首次播放/录音时再 start。
#if TARGET_OS_MACCATALYST
    // 会话分类只在 connect 时设一次 Playback（此时不碰 inputNode）；startMic 授权后再切
    // PlayAndRecord（实测 Playback 下触碰 inputNode 会让引擎直接启动失败）。
    [self applyAudioSessionCategory:AVAudioSessionCategoryPlayback];
#endif
}

/// 懒启动音频引擎（首帧下行/试听/录音前调用）。引擎已在运行则为空操作。
- (void)ensureAudioEngine {
    if (!self.audioEngine) [self setupAudio];
    if (self.audioEngine.isRunning) return;
    [self.audioEngine prepare];
    NSError *err = nil;
    if (![self.audioEngine startAndReturnError:&err]) {
        NSLog(@"[XiaoZhi] audio engine start 失败: %@", err);
        return;
    }
    XZPlayerPlay(self.playerNode);
}

#if TARGET_OS_MACCATALYST
/// 设置 AVAudioSession category 并确保激活（运行中切换由引擎下次 start 生效）。
- (void)applyAudioSessionCategory:(AVAudioSessionCategory)category {
    NSError *err = nil;
    AVAudioSession *session = [AVAudioSession sharedInstance];
    if ([category isEqualToString:AVAudioSessionCategoryPlayAndRecord]) {
        [session setCategory:category withOptions:AVAudioSessionCategoryOptionDefaultToSpeaker error:&err];
    } else {
        [session setCategory:category error:&err];
    }
    if (err) NSLog(@"[XiaoZhi] audio session category error: %@", err);
    err = nil;
    [session setActive:YES error:&err];
    if (err) NSLog(@"[XiaoZhi] audio session active error: %@", err);
}
#endif

/// 下行采样率变化（server hello）时重建 player 的连接格式，保证连接与 buffer 采样率一致。
- (void)reconnectPlayerForSampleRate:(uint32_t)sr {
    if (!self.audioEngine || !self.playerNode) return;
    AVAudioFormat *f = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                        sampleRate:sr
                                                          channels:1
                                                       interleaved:NO];
    [self.audioEngine disconnectNodeOutput:self.playerNode];
    XZEngineConnect(self.audioEngine, self.playerNode, self.audioEngine.mainMixerNode, f);
    if (self.audioEngine.isRunning) XZPlayerPlay(self.playerNode); // 未启动则留给懒启动
    NSLog(@"[XiaoZhi] player 重连为 %.0fHz/1ch（下行采样率变化）", f.sampleRate);
}

/// 麦克风权限查询/申请：completion(ok) 必被调用（授权/拒绝/受限都回调），
/// 调用方据此回报 started / error 给 Kotlin（不再靠 800ms 猜测延时判断录音是否开始）。
- (void)requestMicPermission:(void (^)(BOOL ok))completion {
    // 麦克风权限：用 AVCaptureDevice（Catalyst 13+/macOS 10.14+ 通用；实测 macabi 可编译）。
    // ⚠️ 两条硬前提（此前踩坑）：
    //   ① Info.plist 必须有 NSMicrophoneUsageDescription（缺失时 TCC 直接拒绝，inputNode
    //      报 0 声道 → AUIOBase "no channels in channel map" + AudioConverter -50 刷屏）；
    //   ② 必须等用户授权后再访问 inputNode 的格式（未授权时格式无效）。
    AVAuthorizationStatus st = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
    if (st == AVAuthorizationStatusAuthorized) {
        completion(YES);
        return;
    }
    if (st == AVAuthorizationStatusDenied || st == AVAuthorizationStatusRestricted) {
        NSLog(@"[XiaoZhi] 麦克风权限被拒绝：系统设置 → 隐私与安全性 → 麦克风，勾选本 App 后重试");
        completion(NO);
        return;
    }
    [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL ok) {
        dispatch_async(dispatch_get_main_queue(), ^{
            if (!ok) NSLog(@"[XiaoZhi] 麦克风权限被拒绝：系统设置 → 隐私与安全性 → 麦克风，勾选本 App 后重试");
            completion(ok);
        });
    }];
}

/// 启动麦克风采集。返回 YES 表示 tap 安装且引擎成功启动（录音真的开始了）。
- (BOOL)startMic {
    if (!self.audioEngine) [self setupAudio];
    // 授权后才切 PlayAndRecord（startAsr 的授权回调先行调用）：
    // ⚠️ 实测（本机 Catalyst 复现）在 Playback 分类下触碰 inputNode 会让引擎直接
    //    startAndReturnError 失败（coreaudio 560227702）——必须先切分类再碰输入节点。
#if TARGET_OS_MACCATALYST
    [self applyAudioSessionCategory:AVAudioSessionCategoryPlayAndRecord];
    AVAudioSession *session = [AVAudioSession sharedInstance];
    if (![session.category isEqualToString:AVAudioSessionCategoryPlayAndRecord]) {
        NSLog(@"[XiaoZhi] 会话分类切换未生效（当前 %@），放弃本次录音", session.category);
        return NO;
    }
#endif
    // 懒启动的引擎此时可能未运行；切分类后重建输入路径（stop→重读格式→installTap→start）
    [self.audioEngine stop];
    self.micNode = self.audioEngine.inputNode;
    AVAudioFormat *fmt = [self.micNode outputFormatForBus:0];
    NSLog(@"[XiaoZhi] mic format: %.0f Hz, %u ch", fmt.sampleRate, (unsigned)fmt.channelCount);
    if (fmt.channelCount == 0) {
        NSLog(@"[XiaoZhi] inputNode 仍为 0 声道（权限未生效或无输入设备），放弃本次录音");
        return NO;
    }
    // 每帧 48k 样本数按实际麦克风采样率计算（通常 48000 → 2880）
    self.micFrameSamples = (UInt32)llround(fmt.sampleRate * kFrameDurationMs / 1000.0);
    self.micAccum = [NSMutableData data];
    // 用硬件原生格式安装 tap（不指定 fmt 避免 -50 转换失败），在回调里重采样。
    // 新 SDK 的 installTap 带 error 出参（Catalyst 27+），旧 API 回退（功能一致）。
    __weak typeof(self) weak = self;
    AVAudioNodeTapBlock tapBlock = ^(AVAudioPCMBuffer * _Nonnull buffer, AVAudioTime * _Nonnull when) {
        typeof(self) strong = weak;
        if (!strong || !strong.recording) return;
        [strong appendMicBuffer:buffer];
    };
    if (@available(macCatalyst 27.0, *)) {
        NSError *tapErr = nil;
        if (![self.micNode installTapOnBus:0 bufferSize:1024 format:fmt error:&tapErr block:tapBlock]) {
            NSLog(@"[XiaoZhi] installTap 失败: %@", tapErr);
        }
    } else {
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        [self.micNode installTapOnBus:0 bufferSize:1024 format:fmt block:tapBlock];
#pragma clang diagnostic pop
    }
    NSError *err = nil;
    if (![self.audioEngine startAndReturnError:&err]) {
        NSLog(@"[XiaoZhi] start engine error: %@", err);
        return NO;
    }
    return YES;
}

// 麦克风回调：把任意长度/采样率的 float32 单声道缓冲归一到 48k 后：
//   ① 累积到整帧编码上行（AudioConverter 每次只吃整数帧，攒满 2880 样本发一帧）；
//   ② 降采样 24k 追加 recordingPcm（本地试听/波形，与 TTS 缓冲同格式）。
- (void)appendMicBuffer:(AVAudioPCMBuffer *)buffer {
    if (buffer.frameLength == 0 || buffer.floatChannelData == NULL) return;
    float *src = buffer.floatChannelData[0];
    UInt32 n = buffer.frameLength;
    if (buffer.format.channelCount > 1) {
        // 多声道混单：直接取第 0 声道即可（设备麦克风为单声道，混音场景少见）
    }
    double sr = buffer.format.sampleRate;
    // 统一归一到 48k（非常规采样率线性插值）
    float *unit48 = src;
    UInt32 n48 = n;
    BOOL owned = NO;
    if (fabs(sr - 48000.0) > 1.0) {
        n48 = (UInt32)(n * 48000.0 / sr);
        unit48 = malloc(n48 * sizeof(float));
        owned = YES;
        for (UInt32 i = 0; i < n48; i++) {
            double pos = i * sr / 48000.0;
            UInt32 i0 = (UInt32)pos;
            double frac = pos - i0;
            float a = src[i0 < n ? i0 : n - 1];
            float b = src[(i0 + 1) < n ? (i0 + 1) : (n - 1)];
            unit48[i] = a + (b - a) * (float)frac;
        }
    }
    // ① 上行编码队列（48k 整帧）
    [self.micAccum appendBytes:unit48 length:n48 * sizeof(float)];
    // ② 本地试听缓冲：48k → 24k（2:1 线性插值，试听音质足够）
    UInt32 n24 = n48 / 2;
    if (n24 > 0) {
        float *buf24 = malloc(n24 * sizeof(float));
        for (UInt32 i = 0; i < n24; i++) {
            double pos = i * 2.0;
            UInt32 i0 = (UInt32)pos;
            double frac = pos - i0;
            float a = unit48[i0 < n48 ? i0 : n48 - 1];
            float b = unit48[(i0 + 1) < n48 ? (i0 + 1) : (n48 - 1)];
            buf24[i] = a + (b - a) * (float)frac;
        }
        [self.recordingPcm appendBytes:buf24 length:n24 * sizeof(float)];
        self.recSamples24k += n24; // 只增计数：供 getAudioState 回报「录音中已录秒数」
        free(buf24);
    }
    if (owned) free(unit48);

    // 攒满整帧就编码发送（可能一次回调攒出多帧）
    while (self.micAccum.length >= self.micFrameSamples * sizeof(float)) {
        NSData *frame = [self.micAccum subdataWithRange:NSMakeRange(0, self.micFrameSamples * sizeof(float))];
        [self.micAccum replaceBytesInRange:NSMakeRange(0, frame.length) withBytes:NULL length:0];
        NSData *opus = [self encodeOpus:frame];
        if (opus.length) [self sendBinary:opus];
    }
}

- (void)stopMic {
    if (self.micNode) { [self.micNode removeTapOnBus:0]; self.micNode = nil; }
    self.micAccum = nil;
}

- (void)sendBinary:(NSData *)data {
    if (!self.webSocket || data.length == 0) return;
    // 真实 API：sendMessage:completionHandler:（无 sendData:）
    [self.webSocket sendMessage:[[NSURLSessionWebSocketMessage alloc] initWithData:data]
              completionHandler:^(NSError * _Nullable e) {
        if (e) NSLog(@"[XiaoZhi] send binary error: %@", e);
    }];
}

- (void)playPcm:(NSData *)pcm {
    if (!self.playerNode || pcm.length == 0) return;
    [self ensureAudioEngine]; // 懒启动：首帧下行到达时才拉引擎（connect 阶段零音频活动）
    uint32_t sr = self.downlinkSampleRate ?: kDownlinkSampleRate;
    AVAudioFormat *fmt = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                        sampleRate:sr
                                                          channels:1
                                                       interleaved:NO];
    AVAudioFrameCount frames = (AVAudioFrameCount)(pcm.length / sizeof(float));
    AVAudioPCMBuffer *buf = [[AVAudioPCMBuffer alloc] initWithPCMFormat:fmt frameCapacity:frames];
    buf.frameLength = frames;
    memcpy(buf.floatChannelData[0], pcm.bytes, pcm.length);
    // 60ms 帧逐帧调度（buffer 内 hasValidFrames 由 AVAudioPlayerNode 顺序播放）
    [self.playerNode scheduleBuffer:buf completionHandler:nil];
}

#pragma mark - Opus 编解码（系统 AudioToolbox：kAudioFormatOpus，实测 Catalyst 可用）

// 编码输入回调的状态：转换器可能多次回调索取输入，必须记录“已消费”位置，
// 否则第二次回调会把同一帧重复喂入（产生重复音频）。逐帧调用场景下一帧即返回。
struct XZEncFeed {
    const void *bytes;
    UInt32 totalFrames;   // 本帧总样本数
    UInt32 consumed;      // 已喂入样本数
};

static OSStatus XZEncodeInputProc(AudioConverterRef conv, UInt32 *ioNumPackets, AudioBufferList *ioData,
                                  AudioStreamPacketDescription **outDesc, void *user) {
    (void)conv; (void)outDesc;
    struct XZEncFeed *feed = (struct XZEncFeed *)user;
    UInt32 remain = feed->totalFrames - feed->consumed;
    UInt32 want = *ioNumPackets;
    if (remain == 0 || want == 0) { *ioNumPackets = 0; return noErr; }
    UInt32 give = remain < want ? remain : want;
    ioData->mNumberBuffers = 1;
    ioData->mBuffers[0].mNumberChannels = 1;
    ioData->mBuffers[0].mDataByteSize = give * sizeof(float);
    ioData->mBuffers[0].mData = (void *)((const float *)feed->bytes + feed->consumed);
    feed->consumed += give;
    *ioNumPackets = give;
    return noErr;
}

- (AudioConverterRef)createPcmToOpusConverterFrom:(double)srcRate to:(double)dstRate {
    AudioStreamBasicDescription s = {0}, d = {0};
    s.mSampleRate = srcRate;
    s.mFormatID = kAudioFormatLinearPCM;
    s.mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked;
    s.mChannelsPerFrame = 1;
    s.mBitsPerChannel = 32;
    s.mBytesPerFrame = 4;
    s.mBytesPerPacket = 4;
    s.mFramesPerPacket = 1;
    d.mSampleRate = dstRate;
    d.mFormatID = kAudioFormatOpus;
    d.mChannelsPerFrame = 1;
    d.mFramesPerPacket = (UInt32)(dstRate * kFrameDurationMs / 1000.0);
    AudioConverterRef conv = NULL;
    AudioConverterNew(&s, &d, &conv);
    if (conv) {
        // 码率对齐 xiaozhi-esp32 固件：其上行编码器用 ESP_OPUS_BITRATE_AUTO（= libopus
        // OPUS_AUTO = -1000），16k/mono/AUDIO 下等价 40 kbps（本机 libopus 实测）。
        // 历史值 24 kbps 是随手定的，比 ESP 窄 40%，压缩损失更明显——统一到 40k。
        // （ESP 的 complexity=0 / VBR / DTX 是 esp_opus_enc 专属参数，AudioToolbox 面
        //   不暴露；码率一致是本端能对齐的关键项。）
        UInt32 bitrate = dstRate >= 24000 ? 32000 : 40000;
        AudioConverterSetProperty(conv, kAudioConverterEncodeBitRate, sizeof(bitrate), &bitrate);
    }
    return conv;
}

// 麦克风 48k 单声道 float32 → Opus 16k 一帧
- (NSData *)encodeOpus:(NSData *)pcm48 {
    if (!self.uplinkEncoder) {
        self.uplinkEncoder = [self createPcmToOpusConverterFrom:48000 to:kUplinkSampleRate];
        if (!self.uplinkEncoder) { NSLog(@"[XiaoZhi] 上行 Opus 编码器创建失败"); return nil; }
    }
    struct XZEncFeed feed = { pcm48.bytes, (UInt32)(pcm48.length / sizeof(float)), 0 };
    UInt8 buf[4096];
    AudioBufferList out = {0};
    out.mNumberBuffers = 1;
    out.mBuffers[0].mNumberChannels = 1;
    out.mBuffers[0].mDataByteSize = sizeof(buf);
    out.mBuffers[0].mData = buf;
    AudioStreamPacketDescription pd = {0};
    UInt32 ioPkts = 1;
    OSStatus st = AudioConverterFillComplexBuffer(self.uplinkEncoder, XZEncodeInputProc,
                                                   &feed, &ioPkts, &out, &pd);
    if (st != noErr || ioPkts == 0 || pd.mDataByteSize == 0) {
        NSLog(@"[XiaoZhi] 上行编码失败: st=%d pkts=%u", (int)st, (unsigned)ioPkts);
        return nil;
    }
    return [NSData dataWithBytes:buf length:pd.mDataByteSize];
}

struct XZDecFeed {
    const void *bytes;
    UInt32 size;
    UInt32 frames;      // 包内帧数（= 采样率×帧长/1000，必须显式给：converter 不会从 Opus 头推导）
    int provided;
    AudioStreamPacketDescription desc;
};

static OSStatus XZDecodeInputProc(AudioConverterRef conv, UInt32 *ioNumPackets, AudioBufferList *ioData,
                                  AudioStreamPacketDescription **outDesc, void *user) {
    (void)conv;
    struct XZDecFeed *feed = (struct XZDecFeed *)user;
    if (feed->provided) {
        // EOF 信号（输入耗尽）：必须把 ioData 全部零化后再返回 0 包。
        // 不置零时转换器会把 ioData 里遗留/预填的字节数当成"已提供但无包描述"的输入，
        // 打印 "61440 bytes of input provided, but packet descriptions (0) only
        // account for 0 bytes"（实机日志刷屏的另一来源）。
        ioData->mNumberBuffers = 1;
        ioData->mBuffers[0].mNumberChannels = 1;
        ioData->mBuffers[0].mDataByteSize = 0;
        ioData->mBuffers[0].mData = NULL;
        *ioNumPackets = 0;
        return noErr;
    }
    ioData->mNumberBuffers = 1;
    ioData->mBuffers[0].mNumberChannels = 1;
    ioData->mBuffers[0].mDataByteSize = feed->size;
    ioData->mBuffers[0].mData = (void *)feed->bytes;
    feed->desc.mStartOffset = 0;
    feed->desc.mDataByteSize = feed->size;
    // ⚠️ 必须显式给帧数：实测填 0 时 converter 按 CBR（mBytesPerPacket=0）解释压缩输入，
    //    报 "packet descriptions (0) only account for 0 bytes" 且 0 帧输出。
    feed->desc.mVariableFramesInPacket = feed->frames;
    if (outDesc) *outDesc = &feed->desc;
    feed->provided = 1;
    *ioNumPackets = 1;
    return noErr;
}

// 下行 Opus(server 采样率) → float32 LPCM（单声道），供玩家节点播放
- (void)rebuildDownlinkDecoder {
    if (self.downlinkDecoder) { AudioConverterDispose(self.downlinkDecoder); self.downlinkDecoder = NULL; }
    uint32_t sr = self.downlinkSampleRate ?: kDownlinkSampleRate;
    AudioStreamBasicDescription s = {0}, d = {0};
    s.mSampleRate = sr;
    s.mFormatID = kAudioFormatOpus;
    s.mChannelsPerFrame = 1;
    s.mFramesPerPacket = (UInt32)(sr * kFrameDurationMs / 1000.0);
    d.mSampleRate = sr;
    d.mFormatID = kAudioFormatLinearPCM;
    d.mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked;
    d.mChannelsPerFrame = 1;
    d.mBitsPerChannel = 32;
    d.mBytesPerFrame = 4;
    d.mBytesPerPacket = 4;
    d.mFramesPerPacket = 1;
    AudioConverterRef conv = NULL;
    OSStatus st = AudioConverterNew(&s, &d, &conv);
    if (st != noErr) {
        NSLog(@"[XiaoZhi] 下行 Opus 解码器创建失败: %d", (int)st);
        self.downlinkDecoder = NULL;
        return;
    }
    // ⚠️ 关键：禁用 pre-skip 修剪（primeInfo leadingFrames=0）。默认行为下首包解出
    //   1380 帧（<整帧 1440）→ 转换器为「凑满请求量」二次索要输入 → 被拒（输入耗尽）
    //   → 进入 EOF 粘滞：之后所有 Fill 直接返回 0 帧且不再回调输入（只能 Reset 才能解）。
    //   禁用修剪后每包都精确产出整帧 → 单次索要即满足 → 不触发粘滞 →
    //   整段语音可全程复用同一解码器（帧间 overlap 状态跨包保留 = 无缝衔接）。
    //   本机对照实验：连续 5 包全 1440 帧、输入回调各 1 次；拼接流包边界二阶差分
    //   仅为帧内的 0.1x（无爆音/断点证据）。
    AudioConverterPrimeInfo pi = { .leadingFrames = 0, .trailingFrames = 0 };
    AudioConverterSetProperty(conv, kAudioConverterPrimeInfo, sizeof(pi), &pi);
    self.downlinkDecoder = conv;
}

- (NSData *)decodeOpus:(NSData *)opus {
    if (!self.downlinkDecoder) [self rebuildDownlinkDecoder];
    if (!self.downlinkDecoder) return nil;
    // ⚠️ 这里**不做**每包 AudioConverterReset（历史实现为绕过 EOF 粘滞曾每包 Reset）：
    //    每包 Reset 会丢弃解码器的帧间 overlap/预测状态，每个 60ms 边界都引入一次状态
    //    断层——实听「声音不连续」的一条来源。现在禁用 pre-skip 修剪后粘滞不再触发
    //    （见 rebuildDownlinkDecoder 注释），解码器整段连续工作；仅在新一次合成开始时
    //    重置一次（与编码端「每句一个新编码器」的状态边界对齐，见 speak:）。
    uint32_t sr = self.downlinkSampleRate ?: kDownlinkSampleRate;
    UInt32 frameSamples = (UInt32)(sr * kFrameDurationMs / 1000.0); // 24k/60ms = 1440
    float pcm[8192];
    AudioBufferList out = {0};
    out.mNumberBuffers = 1;
    out.mBuffers[0].mNumberChannels = 1;
    out.mBuffers[0].mDataByteSize = sizeof(pcm);
    out.mBuffers[0].mData = pcm;
    // 请求整帧输出（与单包产出精确匹配：修剪已禁用，每包恰产出 frameSamples 帧）
    UInt32 ioFrames = frameSamples;
    struct XZDecFeed feed = {
        opus.bytes,
        (UInt32)opus.length,
        frameSamples, // 包内帧数，必须显式给（见 input proc 注释）
        0,
        {0},
    };
    // 输出为 LPCM（非分包格式），outPacketDescription 必须为 NULL（传非 NULL 数组无益且语义错误）
    OSStatus st = AudioConverterFillComplexBuffer(self.downlinkDecoder, XZDecodeInputProc,
                                                   &feed, &ioFrames, &out, NULL);
    if (st != noErr || ioFrames == 0) {
        // 解码失败自愈：重建解码器（重建设置 primeInfo）；日志前 3 次 + 每 20 次
        self.decodeFailCount++;
        self.ttsFailCount++;
        if (self.decodeFailCount <= 3 || self.decodeFailCount % 20 == 1) {
            NSLog(@"[XiaoZhi] 下行解码失败(连续 %lu): st=%d frames=%u，已重建解码器",
                  (unsigned long)self.decodeFailCount, (int)st, (unsigned)ioFrames);
        }
        [self rebuildDownlinkDecoder];
        return nil;
    }
    self.decodeFailCount = 0;
    self.ttsDecodedPackets++;
    if (!self.loggedFirstDecode) {
        self.loggedFirstDecode = YES;
        NSLog(@"[XiaoZhi] 首个下行包解码成功: %u bytes → %u frames @ %uHz",
              (unsigned)opus.length, (unsigned)ioFrames, sr);
    }
    return [NSData dataWithBytes:pcm length:ioFrames * sizeof(float)];
}

#pragma mark - 回调

- (void)invoke:(id)callback result:(NSDictionary *)result success:(BOOL)success error:(NSString *)error {
    if (!callback) return;
    ((void (^)(id, BOOL, NSString *))callback)(result ?: @{}, success, error ?: nil);
}

@end
