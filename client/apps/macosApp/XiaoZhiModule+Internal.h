//! XiaoZhiModule 的**内部共享头**（类别文件与主实现共用，非对外 API）。
//!
//! 为什么需要：XiaoZhiModule.m 原为单文件（900+ 行），按职责拆分为
//! 主实现（Module 方法/WebSocket/试听）+ `XiaoZhiModule+Audio.m`（采集与播放）
//! + `XiaoZhiModule+Opus.m`（Opus 编解码）。ObjC 类别不能声明实例变量，
//! 故把**私有属性/跨文件方法/共享常量与工具函数**集中在此头：
//! 各拆分文件 import 本头即可访问（属性实现仍在主实现的编译单元）。
//!
//! ⚠️ 本头**只给本类内部用**，不要 import 进 Kotlin/桥接侧头文件。

#import "XiaoZhiModule.h"
#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <QuartzCore/QuartzCore.h> // CACurrentMediaTime（试听进度推算）
#import <UIKit/UIKit.h>           // UIPasteboard（复制到系统剪贴板）

NS_ASSUME_NONNULL_BEGIN

extern const uint32_t kUplinkSampleRate;   // 上行（麦克风）采样率，与 server 协商一致
extern const uint32_t kDownlinkSampleRate; // 下行（TTS）默认采样率，实际以 server hello 为准
extern const uint32_t kFrameDurationMs;    // 上下行帧长，与 server 一致

// ===== 新版 API 兼容封装（Xcode 27 SDK 把 connect/play/installTap 改为带 error 的版本）=====
// 部署目标 macOS 12（Catalyst iOS 15），新 API 需 Catalyst 27+：可用则用新 API（并回报错误），
// 否则回退旧 API（仅弃用、功能一致，局部抑制弃用告警保持构建零警告）。
static inline BOOL XZEngineConnect(AVAudioEngine *engine, AVAudioNode *src, AVAudioNode *dst, AVAudioFormat *fmt) {
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

static inline void XZPlayerPlay(AVAudioPlayerNode *player) {
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

// ===== 跨文件方法声明（实现分布在类别文件；主实现调用时不产生 "no visible @interface" 告警）=====
// XiaoZhiModule+Audio.m
- (void)setupAudio;
- (void)ensureAudioEngine;
- (void)requestMicPermission:(void (^)(BOOL ok))completion;
- (void)reconnectPlayerForSampleRate:(uint32_t)sr;
- (BOOL)startMic;
- (void)stopMic;
- (void)playPcm:(NSData *)pcm;
- (void)sendBinary:(NSData *)data;
// XiaoZhiModule+Opus.m
- (NSData *)encodeOpus:(NSData *)pcm48;
- (void)rebuildDownlinkDecoder;
- (NSData *)decodeOpus:(NSData *)opus;
@end

NS_ASSUME_NONNULL_END
