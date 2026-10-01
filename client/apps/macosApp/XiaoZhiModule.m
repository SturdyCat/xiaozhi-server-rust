#import "XiaoZhiModule.h"
#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <QuartzCore/QuartzCore.h> // CACurrentMediaTime（试听进度推算）

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
@property (nonatomic, copy, nullable) id speakCallback; // 待 tts stop 时回调
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

    // hello：version=1 → 上行裸 Opus（v1）；audio_params 声明本端 16k/opus
    [self sendJSON:@{
        @"type": @"hello",
        @"version": @(1),
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
- (void)startAsr:(NSDictionary *)args {
    self.asrCallback = args[KR_CALLBACK_KEY]; // 保留，待服务端回 stt 时回调
    self.recordingPcm = [NSMutableData data];
    [self requestMicPermission:^{
        self.recording = YES;
        [self sendJSON:@{@"type": @"asr_test", @"action": @"start"}];
        [self startMic];
    }];
}

// stopRecording() → 仅停止采集（保留录音缓冲供试听），不发 asr_test stop。
// 「发送识别」由 sendAsr 触发——录音与识别解耦，中间可反复试听。
- (void)stopRecording:(NSDictionary *)args {
    self.recording = NO;
    [self stopMic];
    id callback = args[KR_CALLBACK_KEY];
    double seconds = self.recordingPcm.length / (24000.0 * sizeof(float));
    [self invoke:callback result:@{@"seconds": @(seconds)} success:YES error:nil];
}

// sendAsr() → 发送 asr_test stop；服务端对整段（此前流式上传的）做一次性识别并回 stt
// （stt 经既有 asrCallback 通道回到 Kotlin）。
- (void)sendAsr:(NSDictionary *)args {
    [self sendJSON:@{@"type": @"asr_test", @"action": @"stop"}];
}

// speak(text, speaker, lang, speed) → 发送 tts_test；服务端合成并流式下发 Opus 音频。
// 下行 PCM 同时累积到 ttsPcm，tts stop 后可供试听/波形（playTts）。
- (void)speak:(NSDictionary *)args {
    NSDictionary *params = [self parseParams:args[KR_PARAM_KEY]];
    self.speakCallback = args[KR_CALLBACK_KEY];
    self.ttsPcm = [NSMutableData data];
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

// getAudioState(source) → {wave:[64 桶峰值 0~1], duration, playing, position}
// Kotlin 侧 100ms 轮询驱动播放进度可视化；source: "recording" | "tts"
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
    [self invoke:callback result:@{
        @"wave": wave ?: @[],
        @"duration": @(duration),
        @"playing": @(playing),
        @"position": @(position),
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
            [self invoke:self.asrCallback result:@{@"text": t} success:YES error:nil];
            self.asrCallback = nil; // one-shot
        }
        return;
    }
    if ([type isEqualToString:@"tts"]) {
        NSString *state = dict[@"state"];
        if ([state isEqualToString:@"stop"] && self.speakCallback) {
            [self invoke:self.speakCallback result:@{@"state": @"stop"} success:YES error:nil];
            self.speakCallback = nil;
        }
        return;
    }
}

- (void)handleBinary:(NSData *)opus {
    if (opus.length == 0) return;
    NSData *pcm = [self decodeOpus:opus];
    if (!pcm.length) return;
    // 累积到 TTS 缓冲（24k float32，与下行采样率一致），tts stop 后供试听/波形
    [self.ttsPcm appendData:pcm];
    [self playPcm:pcm];
}

#pragma mark - 音频采集 / 播放

- (void)setupAudio {
    if (self.audioEngine) return;
    self.audioEngine = [[AVAudioEngine alloc] init];
    self.playerNode = [[AVAudioPlayerNode alloc] init];
    [self.audioEngine attachNode:self.playerNode];
    // ⚠️ player→mainMixer 必须**显式指定**与调度 buffer 相同的声道数（下行单声道）。
    //    实测（本机 AVAudioEngine）：format:nil 时 mixer 协商为 48000Hz/2ch，调度
    //    1ch buffer 直接抛 "required condition is false: _outputFormat.channelCount ==
    //    buffer.format.channelCount"。采样率也会按连接解释（不抛异常但变速），
    //    故连接采样率与下行采样率必须一致（hello 后如变化走 reconnectPlayerForSampleRate:）。
    AVAudioFormat *playerFormat = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                                   sampleRate:(self.downlinkSampleRate ?: kDownlinkSampleRate)
                                                                     channels:1
                                                                  interleaved:NO];
    XZEngineConnect(self.audioEngine, self.playerNode, self.audioEngine.mainMixerNode, playerFormat);
#if TARGET_OS_MACCATALYST
    // Mac Catalyst：AVAudioSession 可用（iOS API 面）。PlayAndRecord + 默认扬声器，
    // 供 TTS 播放与麦克风采集共存（实测 macabi 目标下这些 API 均可编译/可用）。
    NSError *err = nil;
    AVAudioSession *session = [AVAudioSession sharedInstance];
    [session setCategory:AVAudioSessionCategoryPlayAndRecord
             withOptions:AVAudioSessionCategoryOptionDefaultToSpeaker
                   error:&err];
    if (err) NSLog(@"[XiaoZhi] audio session category error: %@", err);
    err = nil;
    [session setActive:YES error:&err];
    if (err) NSLog(@"[XiaoZhi] audio session active error: %@", err);
#endif
    [self.audioEngine prepare];
    NSError *startErr = nil;
    [self.audioEngine startAndReturnError:&startErr];
    if (startErr) NSLog(@"[XiaoZhi] audio engine error: %@", startErr);
    XZPlayerPlay(self.playerNode);
}

/// 下行采样率变化（server hello）时重建 player 的连接格式，保证连接与 buffer 采样率一致。
- (void)reconnectPlayerForSampleRate:(uint32_t)sr {
    if (!self.audioEngine || !self.playerNode) return;
    AVAudioFormat *f = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                        sampleRate:sr
                                                          channels:1
                                                       interleaved:NO];
    [self.audioEngine disconnectNodeOutput:self.playerNode];
    XZEngineConnect(self.audioEngine, self.playerNode, self.audioEngine.mainMixerNode, f);
    XZPlayerPlay(self.playerNode);
    NSLog(@"[XiaoZhi] player 重连为 %.0fHz/1ch（下行采样率变化）", f.sampleRate);
}

- (void)requestMicPermission:(void (^)(void))granted {
    // 麦克风权限：用 AVCaptureDevice（Catalyst 13+/macOS 10.14+ 通用；实测 macabi 可编译）。
    // ⚠️ 两条硬前提（此前踩坑）：
    //   ① Info.plist 必须有 NSMicrophoneUsageDescription（缺失时 TCC 直接拒绝，inputNode
    //      报 0 声道 → AUIOBase "no channels in channel map" + AudioConverter -50 刷屏）；
    //   ② 必须等用户授权后再访问 inputNode 的格式（未授权时格式无效）。
    AVAuthorizationStatus st = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
    if (st == AVAuthorizationStatusAuthorized) {
        granted();
        return;
    }
    if (st == AVAuthorizationStatusDenied || st == AVAuthorizationStatusRestricted) {
        NSLog(@"[XiaoZhi] 麦克风权限被拒绝：系统设置 → 隐私与安全性 → 麦克风，勾选本 App 后重试");
        return;
    }
    [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL ok) {
        dispatch_async(dispatch_get_main_queue(), ^{
            if (!ok) {
                NSLog(@"[XiaoZhi] 麦克风权限被拒绝：系统设置 → 隐私与安全性 → 麦克风，勾选本 App 后重试");
                return;
            }
            granted();
        });
    }];
}

- (void)startMic {
    self.micNode = self.audioEngine.inputNode;
    AVAudioFormat *fmt = [self.micNode outputFormatForBus:0];
    NSLog(@"[XiaoZhi] mic format: %.0f Hz, %u ch", fmt.sampleRate, (unsigned)fmt.channelCount);
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
    [self.audioEngine startAndReturnError:&err];
    if (err) NSLog(@"[XiaoZhi] start engine error: %@", err);
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
        // bitrate 显式指定，避免默认值在部分采样率下生成失败
        UInt32 bitrate = dstRate >= 24000 ? 32000 : 24000;
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
    int provided;
    AudioStreamPacketDescription desc;
};

static OSStatus XZDecodeInputProc(AudioConverterRef conv, UInt32 *ioNumPackets, AudioBufferList *ioData,
                                  AudioStreamPacketDescription **outDesc, void *user) {
    (void)conv;
    struct XZDecFeed *feed = (struct XZDecFeed *)user;
    if (feed->provided) { *ioNumPackets = 0; return noErr; }
    ioData->mNumberBuffers = 1;
    ioData->mBuffers[0].mNumberChannels = 1;
    ioData->mBuffers[0].mDataByteSize = feed->size;
    ioData->mBuffers[0].mData = (void *)feed->bytes;
    feed->desc.mStartOffset = 0;
    feed->desc.mDataByteSize = feed->size;
    feed->desc.mVariableFramesInPacket = 0; // 由解码器按 Opus 头推导
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
    } else {
        self.downlinkDecoder = conv;
    }
}

- (NSData *)decodeOpus:(NSData *)opus {
    if (!self.downlinkDecoder) [self rebuildDownlinkDecoder];
    if (!self.downlinkDecoder) return nil;
    float pcm[8192];
    AudioBufferList out = {0};
    out.mNumberBuffers = 1;
    out.mBuffers[0].mNumberChannels = 1;
    out.mBuffers[0].mDataByteSize = sizeof(pcm);
    out.mBuffers[0].mData = pcm;
    UInt32 ioFrames = 8192;
    struct XZDecFeed feed = { opus.bytes, (UInt32)opus.length, 0, {0} };
    OSStatus st = AudioConverterFillComplexBuffer(self.downlinkDecoder, XZDecodeInputProc,
                                                   &feed, &ioFrames, &out, NULL);
    if (st != noErr || ioFrames == 0) {
        // 单包解码首帧报错时重建解码器（状态异常自愈）
        NSLog(@"[XiaoZhi] 下行解码失败: st=%d frames=%u，重建解码器", (int)st, (unsigned)ioFrames);
        [self rebuildDownlinkDecoder];
        return nil;
    }
    return [NSData dataWithBytes:pcm length:ioFrames * sizeof(float)];
}

#pragma mark - 回调

- (void)invoke:(id)callback result:(NSDictionary *)result success:(BOOL)success error:(NSString *)error {
    if (!callback) return;
    ((void (^)(id, BOOL, NSString *))callback)(result ?: @{}, success, error ?: nil);
}

@end
