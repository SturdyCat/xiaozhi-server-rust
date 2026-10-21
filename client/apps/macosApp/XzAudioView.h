#import <UIKit/UIKit.h>

NS_ASSUME_NONNULL_BEGIN

/**
 * Kuikly 2.29.2 新增 AudioView 的 iOS/macOS(Catalyst) 宿主实现。
 *
 * Kuikly 只提供 KRAudioView 这个「壳」，真正的播放器由宿主注入：
 * 启动时调用 +registerWithKuikly 注册一个 AudioViewCreator，之后 DSL 里的
 * `Audio { src(...) }` 就会创建本类实例，由 AVPlayer 播放 file:// 或 http(s)://。
 */
@interface XzAudioView : UIView

/// 把本类注册进 OpenKuiklyIOSRender 的 KRAudioView（App 启动时调用一次）。
+ (void)registerWithKuikly;

@end

NS_ASSUME_NONNULL_END
