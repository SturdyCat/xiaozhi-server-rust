#import "AppDelegate.h"
#import "XzSvgView.h"
#import <AVFoundation/AVFoundation.h>

@implementation AppDelegate

// UIScene 生命周期下 AppDelegate 只保留应用级初始化；窗口创建已迁至 SceneDelegate。
- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
    // Pinned Kuikly factory discovers XzSvgView by its exact Objective-C class name.
    // Do not replace componentExpandHandler: existing image/color handlers remain intact.
    [XzSvgView setResourceRootURL:NSBundle.mainBundle.resourceURL];

    // Mac Catalyst 下音频会话用于麦克风采集 + TTS 播放。
    NSError *err = nil;
    [[AVAudioSession sharedInstance] setCategory:AVAudioSessionCategoryPlayAndRecord
                                     withOptions:AVAudioSessionCategoryOptionDefaultToSpeaker
                                           error:&err];
    if (err) NSLog(@"[macosApp] audio session error: %@", err);
    return YES;
}

@end
