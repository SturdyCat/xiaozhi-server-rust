#import "SceneDelegate.h"
#import "KuiklyRenderViewController.h"

@implementation SceneDelegate

// UIScene 生命周期：窗口创建必须在这里（新 SDK 强制，AppDelegate 直建 window 启动即报
// "UIScene life cycle is required for apps built with this SDK"）。
- (void)scene:(UIScene *)scene willConnectToSession:(UISceneSession *)session
       options:(UISceneConnectionOptions *)connectionOptions {
    if (![scene isKindOfClass:[UIWindowScene class]]) return;
    UIWindowScene *windowScene = (UIWindowScene *)scene;

    self.window = [[UIWindow alloc] initWithWindowScene:windowScene];

    // Mac 版默认进入 ASR/TTS 测试页（test）；管理后台（config）跨平台，可从测试页跳转或作为独立入口。
    KuiklyRenderViewController *root = [[KuiklyRenderViewController alloc] initWithPageName:@"test"
                                                                                  pageData:@{}];
    UINavigationController *nav = [[UINavigationController alloc] initWithRootViewController:root];
    self.window.rootViewController = nav;
    [self.window makeKeyAndVisible];
}

- (void)sceneDidDisconnect:(UIScene *)scene {
    // 场景断开（如 App 在后台被系统回收资源）；窗口随 scene 释放即可，无需额外处理。
}

@end
