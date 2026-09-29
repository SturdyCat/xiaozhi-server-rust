#import <UIKit/UIKit.h>

NS_ASSUME_NONNULL_BEGIN

@interface KuiklyRenderViewController : UIViewController

/// 创建实例的初始化方法
/// @param pageName 页面名（对应 Kotlin 侧 @Page("xxx") 的 xxx）
/// @param pageData 页面对应的参数（Kotlin 侧通过 pageData.params 获取）
- (instancetype)initWithPageName:(NSString *)pageName pageData:(NSDictionary *)pageData;

@end

NS_ASSUME_NONNULL_END
