#import <UIKit/UIKit.h>
#import "KuiklyRenderViewExportProtocol.h"

NS_ASSUME_NONNULL_BEGIN
// Kuikly viewName is exactly XzSvgView. UIKit implementation is reusable by iOS hosts.
@interface XzSvgView : UIView <KuiklyRenderViewExportProtocol>
// Host injects NSBundle.mainBundle.resourceURL; no network/filesystem URI is accepted.
+ (void)setResourceRootURL:(NSURL *)resourceRootURL;
@end
NS_ASSUME_NONNULL_END
