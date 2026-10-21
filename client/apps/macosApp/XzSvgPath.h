#import <CoreGraphics/CoreGraphics.h>
#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN
// Strict finite SVG numbers: ASCII grammar, bounded coordinates, no units/NaN/Inf.
FOUNDATION_EXPORT BOOL XzSvgReadNumber(const char * _Nonnull * _Nonnull cursor, double *value);
FOUNDATION_EXPORT NSArray<NSNumber *> * _Nullable XzSvgNumbers(NSString *text);
FOUNDATION_EXPORT BOOL XzSvgTransform(NSString * _Nullable text, CGAffineTransform *result);
// Returned paths are immutable, caller-owned; nil means unsupported/malformed syntax.
FOUNDATION_EXPORT CGPathRef _Nullable XzSvgCreatePath(NSString *text) CF_RETURNS_RETAINED;
NS_ASSUME_NONNULL_END
