#import "XiaoZhiModule+Internal.h"

// ⚠️ 参数/回调 key 不能自定义：KR_PARAM_KEY/KR_CALLBACK_KEY 是 OpenKuiklyIOSRender
//    KRBaseModule.h 声明的 extern 常量（实际值为 @"param"/@"callback"）。
//    此前用 #ifndef 兜底定义 @"__kr_param__"，运行期取不到参数（params 恒为空字典），
//    WS 连接实际从未拿到 url（会以 invalid url 失败或此处崩溃）。

// 共享常量定义（声明在 XiaoZhiModule+Internal.h；音频/Opus 类别文件共用）
const uint32_t kUplinkSampleRate = 16000;   // 上行（麦克风）采样率，与 server 协商一致
const uint32_t kDownlinkSampleRate = 24000; // 下行（TTS）默认采样率，实际以 server hello 为准
const uint32_t kFrameDurationMs = 60;       // 上下行帧长，与 server 一致

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

// speak(text, speaker, lang, speed, vcn) → 发送 tts_test；服务端合成并流式下发 Opus 音频。
// vcn（可选）：远程音色覆盖，服务端本次合成即用它（不改变已保存配置）。
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
    // vcn：远程（讯飞）音色覆盖——服务端本次合成即用它（无需先保存配置）；空串 = 用服务端配置
    NSMutableDictionary *msg = [@{
        @"type": @"tts_test",
        @"text": params[@"text"] ?: @"",
        @"speaker": params[@"speaker"] ?: @(0),
        @"lang": params[@"lang"] ?: @"zh",
        @"speed": params[@"speed"] ?: @(1.0)
    } mutableCopy];
    NSString *vcn = params[@"vcn"];
    if ([vcn isKindOfClass:[NSString class]] && vcn.length > 0) {
        msg[@"vcn"] = vcn;
    }
    [self sendJSON:msg];
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

#pragma mark - 回调

- (void)invoke:(id)callback result:(NSDictionary *)result success:(BOOL)success error:(NSString *)error {
    if (!callback) return;
    ((void (^)(id, BOOL, NSString *))callback)(result ?: @{}, success, error ?: nil);
}

@end
