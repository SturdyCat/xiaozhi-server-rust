//! XiaoZhiModule 的 Opus 编解码类别（拆分自 XiaoZhiModule.m）：
//! 系统 AudioToolbox（kAudioFormatOpus）的上行编码/下行解码。关键坑（pre-skip 修剪、
//! EOF 粘滞、显式帧数）见各方法内注释。私有属性与共享常量见 `XiaoZhiModule+Internal.h`。

#import "XiaoZhiModule+Internal.h"

@implementation XiaoZhiModule (Opus)

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

@end
