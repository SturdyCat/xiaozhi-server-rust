#import "XzAudioView.h"
#import <AVFoundation/AVFoundation.h>
#import <OpenKuiklyIOSRender/KRAudioView.h>

/**
 * AudioView 宿主播放器：用 AVPlayer 播 src（file:// 或 http(s)://），
 * 把播放状态/时间变化回馈给 Kuikly AudioEvent（playStateDidChanged / playTimeDidChanged）。
 *
 * ⚠️ 依赖 Kuikly >= 2.29.2（KRAudioView 在该版本新增）；Pod 需先 `pod install`。
 */
@interface XzAudioView () <KRAudioViewProtocol>

@property (nonatomic, strong) AVPlayer *player;
@property (nonatomic, strong, nullable) NSURL *srcURL;
@property (nonatomic, strong, nullable) id timeObserver;

@end

@implementation XzAudioView

// 协议属性由 Kuikly 侧赋值（等号 setter），显式合成以保留读写能力。
@synthesize kra_delegate = _kra_delegate;

+ (void)registerWithKuikly {
    [KRAudioView registerAudioViewCreator:^id<KRAudioViewProtocol>(NSString *src, CGRect frame) {
        return [[XzAudioView alloc] initWithSrc:src frame:frame];
    }];
}

- (instancetype)initWithSrc:(NSString *)src frame:(CGRect)frame {
    self = [super initWithFrame:frame];
    if (self) {
        self.backgroundColor = UIColor.clearColor;
        self.userInteractionEnabled = NO;
        [self loadSrc:src];
        [self observePlayer];
    }
    return self;
}

- (void)loadSrc:(NSString *)src {
    if (src.length == 0) return;
    NSURL *url = [src hasPrefix:@"file://"] || [src hasPrefix:@"http"]
        ? [NSURL URLWithString:src]
        : [NSURL fileURLWithPath:src];
    if (url == nil) return;
    self.srcURL = url;

    AVPlayerItem *item = [AVPlayerItem playerItemWithURL:url];
    if (self.player) {
        [self.player replaceCurrentItemWithPlayerItem:item];
    } else {
        self.player = [AVPlayer playerWithPlayerItem:item];
        if (@available(iOS 10.0, *)) {
            self.player.automaticallyWaitsToMinimizeStalling = NO;
        }
    }
}

- (void)observePlayer {
    [self.player addObserver:self
                  forKeyPath:@"timeControlStatus"
                     options:NSKeyValueObservingOptionNew
                     context:nil];
    [[NSNotificationCenter defaultCenter] addObserver:self
                                             selector:@selector(onPlayToEnd:)
                                                 name:AVPlayerItemDidPlayToEndTimeNotification
                                               object:nil];

    __weak typeof(self) weak = self;
    self.timeObserver = [self.player addPeriodicTimeObserverForInterval:CMTimeMakeWithSeconds(0.1, NSEC_PER_SEC)
                                                                  queue:dispatch_get_main_queue()
                                                             usingBlock:^(CMTime time) {
        typeof(self) strong = weak;
        if (strong == nil) return;
        NSUInteger current = (NSUInteger)(CMTimeGetSeconds(time) * 1000.0);
        NSUInteger total = 0;
        CMTime duration = strong.player.currentItem.duration;
        if (CMTIME_IS_NUMERIC(duration)) {
            total = (NSUInteger)(CMTimeGetSeconds(duration) * 1000.0);
        }
        [strong.kra_delegate playTimeDidChangedWithCurrentTime:current totalTime:total];
    }];
}

- (void)observeValueForKeyPath:(NSString *)keyPath
                      ofObject:(id)object
                        change:(NSDictionary<NSKeyValueChangeKey, id> *)change
                       context:(void *)context {
    if (![keyPath isEqualToString:@"timeControlStatus"]) {
        [super observeValueForKeyPath:keyPath ofObject:object change:change context:context];
        return;
    }
    KRAudioPlayState state = KRAudioPlayStatePaused;
    switch (self.player.timeControlStatus) {
        case AVPlayerTimeControlStatusPlaying:
            state = KRAudioPlayStatePlaying;
            break;
        case AVPlayerTimeControlStatusWaitingToPlayAtSpecifiedRate:
            state = KRAudioPlayStateCaching;
            break;
        case AVPlayerTimeControlStatusPaused:
        default:
            state = KRAudioPlayStatePaused;
            break;
    }
    [self.kra_delegate audioPlayStateDidChangedWithState:state extInfo:@{}];
}

- (void)onPlayToEnd:(NSNotification *)note {
    if (note.object != self.player.currentItem) return;
    [self.kra_delegate audioPlayStateDidChangedWithState:KRAudioPlayStatePlayEnd extInfo:@{}];
}

#pragma mark - KRAudioViewProtocol

- (void)kra_play {
    // KRAudioView 的 src 只绑定一次、播放器不复用（见 Kuikly KRAudioView.m），
    // 而试听文件每次都会被覆盖重写；因此每次 play 用新 item 重载同一 URL，既读新内容也从头播。
    if (self.srcURL) {
        AVPlayerItem *fresh = [AVPlayerItem playerItemWithURL:self.srcURL];
        [self.player replaceCurrentItemWithPlayerItem:fresh];
    }
    [self.player play];
}

- (void)kra_pause {
    [self.player pause];
}

- (void)kra_stop {
    [self.player pause];
    [self.player seekToTime:kCMTimeZero];
}

- (void)kra_setRate:(CGFloat)rate {
    self.player.rate = (float)rate;
}

- (void)kra_seekToTime:(NSUInteger)seekTotime {
    [self.player seekToTime:CMTimeMake((int64_t)seekTotime, 1000)
          toleranceBefore:kCMTimeZero
           toleranceAfter:kCMTimeZero];
}

- (void)dealloc {
    if (self.timeObserver) {
        [self.player removeTimeObserver:self.timeObserver];
    }
    [self.player removeObserver:self forKeyPath:@"timeControlStatus"];
    [[NSNotificationCenter defaultCenter] removeObserver:self];
}

@end
