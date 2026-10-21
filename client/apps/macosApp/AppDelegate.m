#import "AppDelegate.h"
#import "XzAudioView.h"
#import <AVFoundation/AVFoundation.h>

@implementation AppDelegate

// UIScene 生命周期下 AppDelegate 只保留应用级初始化；窗口创建已迁至 SceneDelegate。
- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
    // Kuikly 2.29.2 AudioView：注册宿主播放器（AVPlayer），供测试台 TTS/录音试听以 src URL 播放。
    [XzAudioView registerWithKuikly];

    // Mac Catalyst 下音频会话用于麦克风采集 + TTS 播放。
    NSError *err = nil;
    [[AVAudioSession sharedInstance] setCategory:AVAudioSessionCategoryPlayAndRecord
                                     withOptions:AVAudioSessionCategoryOptionDefaultToSpeaker
                                           error:&err];
    if (err) NSLog(@"[macosApp] audio session error: %@", err);
    return YES;
}

@end
