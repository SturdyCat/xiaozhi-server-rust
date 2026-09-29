#import "XiaoZhiModule.h"
#import <AVFoundation/AVFoundation.h>

// Kuikly 通过这两个 key 从 args 中取参数与回调（来自 OpenKuiklyIOSRender 宏定义）。
// 若集成时实际宏名不同，请以 pod 头文件为准。
#ifndef KR_PARAM_KEY
#define KR_PARAM_KEY @"__kr_param__"
#endif
#ifndef KR_CALLBACK_KEY
#define KR_CALLBACK_KEY @"__kr_callback__"
#endif

static const uint32_t kUplinkSampleRate = 16000;   // 上行（麦克风）采样率，与 server 协商一致
static const uint32_t kDownlinkSampleRate = 24000; // 下行（TTS）默认采样率，实际以 server hello 为准

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
@end

@implementation XiaoZhiModule

+ (NSString *)moduleName { return @"XiaoZhiModule"; }

#pragma mark - Module 方法（方法名与 Kotlin 侧 toNative(methodName=...) 一一对应）

// connect(url, token) → 建立 WS 并发送 hello
- (void)connect:(NSDictionary *)args {
    NSDictionary *params = args[KR_PARAM_KEY] ?: @{};
    id callback = args[KR_CALLBACK_KEY];
    NSString *url = params[@"url"] ?: @"";
    NSString *token = params[@"token"] ?: @"";

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
            @"frame_duration": @(60)
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

// startAsr() → 申请麦克风权限并开始采集，发送 asr_test start；服务端缓冲整段音频（跳过 VAD）
- (void)startAsr:(NSDictionary *)args {
    self.asrCallback = args[KR_CALLBACK_KEY]; // 保留，待服务端回 stt 时回调
    [self requestMicPermission:^{
        self.recording = YES;
        [self sendJSON:@{@"type": @"asr_test", @"action": @"start"}];
        [self startMic];
    }];
}

// stopAsr() → 停止采集，发送 asr_test stop；服务端对整段做一次性识别并回 stt
- (void)stopAsr:(NSDictionary *)args {
    self.recording = NO;
    [self stopMic];
    [self sendJSON:@{@"type": @"asr_test", @"action": @"stop"}];
}

// speak(text, speaker, lang, speed) → 发送 tts_test；服务端合成并流式下发 Opus 音频
- (void)speak:(NSDictionary *)args {
    NSDictionary *params = args[KR_PARAM_KEY] ?: @{};
    self.speakCallback = args[KR_CALLBACK_KEY];
    [self sendJSON:@{
        @"type": @"tts_test",
        @"text": params[@"text"] ?: @"",
        @"speaker": params[@"speaker"] ?: @(0),
        @"lang": params[@"lang"] ?: @"zh",
        @"speed": params[@"speed"] ?: @(1.0)
    }];
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
        if (msg.type == NSURLSessionWebSocketMessageTypeString) {
            [strong handleText:msg.string];
        } else if (msg.type == NSURLSessionWebSocketMessageTypeData) {
            [strong handleBinary:msg.data];
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
        if (ap[@"sample_rate"]) self.downlinkSampleRate = [ap[@"sample_rate"] unsignedIntValue];
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
    // TODO: Opus 解码（downlink 采样率）→ Float32 PCM，再播放。下方为占位直通。
    NSData *pcm = [self decodeOpus:opus];
    [self playPcm:pcm];
}

#pragma mark - 音频采集 / 播放

- (void)setupAudio {
    if (self.audioEngine) return;
    self.audioEngine = [[AVAudioEngine alloc] init];
    self.playerNode = [[AVAudioPlayerNode alloc] init];
    [self.audioEngine attachNode:self.playerNode];
    AVAudioFormat *outFormat = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                                sampleRate:kDownlinkSampleRate
                                                                  channels:1
                                                               interleaved:NO];
    [self.audioEngine connect:self.playerNode to:self.audioEngine.mainMixerNode format:outFormat];
    NSError *err = nil;
    [[AVAudioSession sharedInstance] setCategory:AVAudioSessionCategoryPlayAndRecord
                                     withOptions:AVAudioSessionCategoryOptionDefaultToSpeaker
                                           error:&err];
    [[AVAudioSession sharedInstance] setActive:YES error:&err];
    [self.audioEngine prepare];
    [self.audioEngine startAndReturnError:&err];
    if (err) NSLog(@"[XiaoZhi] audio engine error: %@", err);
    [self.playerNode play];
}

- (void)requestMicPermission:(void (^)(void))granted {
    if (@available(macOS 11.0, *)) {
        [AVAudioSession.sharedInstance requestRecordPermission:^(BOOL ok) {
            dispatch_async(dispatch_get_main_queue(), ^{ if (ok) granted(); });
        }];
    } else {
        granted();
    }
}

- (void)startMic {
    self.micNode = self.audioEngine.inputNode;
    AVAudioFormat *fmt = [self.micNode outputFormatForBus:0];
    __weak typeof(self) weak = self;
    [self.micNode installTapOnBus:0 bufferSize:1024 format:fmt block:^(AVAudioPCMBuffer * _Nonnull buffer, AVAudioTime * _Nonnull when) {
        typeof(self) strong = weak;
        if (!strong || !strong.recording) return;
        float *p = buffer.floatChannelData[0];
        UInt32 n = buffer.frameLength;
        if (n == 0) return;
        NSData *pcm = [NSData dataWithBytes:p length:n * sizeof(float)];
        // TODO: 重采样到 16k 并 Opus 编码（libopus），按 v1（裸 Opus）二进制下发。
        NSData *opus = [strong encodeOpus:pcm];
        [strong sendBinary:opus];
    }];
    NSError *err = nil;
    [self.audioEngine startAndReturnError:&err];
    if (err) NSLog(@"[XiaoZhi] start engine error: %@", err);
}

- (void)stopMic {
    if (self.micNode) { [self.micNode removeTapOnBus:0]; self.micNode = nil; }
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
    AVAudioFormat *fmt = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
                                                        sampleRate:self.downlinkSampleRate ?: kDownlinkSampleRate
                                                          channels:1
                                                       interleaved:NO];
    AVAudioFrameCount frames = (AVAudioFrameCount)(pcm.length / sizeof(float));
    AVAudioPCMBuffer *buf = [[AVAudioPCMBuffer alloc] initWithPCMFormat:fmt frameCapacity:frames];
    buf.frameLength = frames;
    memcpy(buf.floatChannelData[0], pcm.bytes, pcm.length);
    [self.playerNode scheduleBuffer:buf completionHandler:nil];
}

#pragma mark - Opus 编解码占位（需接入真实 Opus 库）

- (NSData *)encodeOpus:(NSData *)pcm {
    // TODO: 接入 libopus（bridging header 或 OpusKit Swift 包）编码 16k 单声道 Float32 → Opus 帧。
    return pcm; // 占位：实际应返回 Opus 帧
}

- (NSData *)decodeOpus:(NSData *)opus {
    // TODO: 解码 Opus → Float32（downlink 采样率），供 playPcm 播放。
    return opus; // 占位
}

#pragma mark - 回调

- (void)invoke:(id)callback result:(NSDictionary *)result success:(BOOL)success error:(NSString *)error {
    if (!callback) return;
    ((void (^)(id, BOOL, NSString *))callback)(result ?: @{}, success, error ?: nil);
}

@end
