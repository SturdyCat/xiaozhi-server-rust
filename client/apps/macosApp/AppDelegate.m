#import "AppDelegate.h"
#import "KuiklyRenderViewController.h"
#import <AVFoundation/AVFoundation.h>

@implementation AppDelegate

- (BOOL)application:(UIApplication *)application didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
    // Mac Catalyst 下音频会话用于麦克风采集 + TTS 播放。
    NSError *err = nil;
    [[AVAudioSession sharedInstance] setCategory:AVAudioSessionCategoryPlayAndRecord
                                     withOptions:AVAudioSessionCategoryOptionDefaultToSpeaker
                                           error:&err];
    if (err) NSLog(@"[macosApp] audio session error: %@", err);

    self.window = [[UIWindow alloc] initWithFrame:UIScreen.mainScreen.bounds];

    // Mac 版默认进入 ASR/TTS 测试页（test）；管理后台（config）跨平台，可从测试页跳转或作为独立入口。
    KuiklyRenderViewController *root = [[KuiklyRenderViewController alloc] initWithPageName:@"test"
                                                                                  pageData:@{}];
    UINavigationController *nav = [[UINavigationController alloc] initWithRootViewController:root];
    self.window.rootViewController = nav;
    [self.window makeKeyAndVisible];
    return YES;
}

@end
