#import <UIKit/UIKit.h>
#import <QuartzCore/QuartzCore.h>

NS_ASSUME_NONNULL_BEGIN
// Immutable, layer-free vector model. Every makeLayer call creates view-owned layers.
@interface XzSvgNode : NSObject
@property (nonatomic, readonly) NSArray<XzSvgNode *> *children;
@property (nonatomic, readonly, nullable) CGPathRef path;
- (CAShapeLayer *)makeLayerWithTint:(nullable UIColor *)tint;
- (void)applyTint:(nullable UIColor *)tint toLayer:(CAShapeLayer *)layer;
@end

@interface XzSvgDocument : NSObject
@property (nonatomic, readonly) CGRect viewBox;
@property (nonatomic, readonly) XzSvgNode *root;
+ (nullable instancetype)parseData:(NSData *)data error:(NSError * _Nullable * _Nullable)error;
@end
NS_ASSUME_NONNULL_END
