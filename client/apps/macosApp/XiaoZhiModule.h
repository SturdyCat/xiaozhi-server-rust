#import <Foundation/Foundation.h>
#import <OpenKuiklyIOSRender/KRBaseModule.h>

NS_ASSUME_NONNULL_BEGIN

/// Kuikly 自定义 Module：桥接 XiaoZhi server 的 ASR/TTS 测试协议。
/// 类名必须与 Kotlin 侧 XiaoZhiModule.moduleName() 返回值一致（"XiaoZhiModule"）。
/// 方法名与 Kotlin 侧 toNative(methodName=...) 一一对应。
@interface XiaoZhiModule : KRBaseModule

@end

NS_ASSUME_NONNULL_END
