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

    // Mac 版进入统一管理台壳（admin，原 router 双壳已废除）：侧边栏 + 内容区，概览/测试台/配置同页切换。
    KuiklyRenderViewController *root = [[KuiklyRenderViewController alloc] initWithPageName:@"admin"
                                                                                  pageData:@{}];
    UINavigationController *nav = [[UINavigationController alloc] initWithRootViewController:root];
    self.window.rootViewController = nav;
    [self.window makeKeyAndVisible];
}

- (void)sceneDidDisconnect:(UIScene *)scene {
    // 场景断开（如 App 在后台被系统回收资源）；窗口随 scene 释放即可，无需额外处理。
}

@end
