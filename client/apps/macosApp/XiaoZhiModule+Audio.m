//! XiaoZhiModule 的音频采集/播放类别（拆分自 XiaoZhiModule.m）：
//! 音频引擎装配与懒启动、会话分类、麦克风权限与采集、试听播放、下行 PCM 播放。
//! 私有属性与共享常量见 `XiaoZhiModule+Internal.h`。

#import "XiaoZhiModule+Internal.h"

@implementation XiaoZhiModule (Audio)

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

@end
