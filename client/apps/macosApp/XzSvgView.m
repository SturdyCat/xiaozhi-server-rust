#import "XzSvgView.h"
#import "XzSvgDocument.h"
#import "UIView+CSS.h"
#import <math.h>

static NSURL *ResourceRoot;
static NSCache<NSString *, XzSvgDocument *> *Documents(void) {
    static NSCache *cache; static dispatch_once_t once;
    dispatch_once(&once, ^{ cache = [NSCache new]; cache.countLimit = 32; cache.totalCostLimit = 2*1024*1024; });
    return cache;
}
static NSString *TrustedRelativePath(NSString *src) {
    // The allowlist is the injected trusted bundled common tree, not arbitrary file URLs.
    if (![src isKindOfClass:NSString.class] || src.length > 240 || ![src hasPrefix:@"assets://common/"] || ![src hasSuffix:@".svg"]) return nil;
    NSString *relative = [src substringFromIndex:@"assets://".length];
    NSString *stem = [relative substringToIndex:relative.length-4];
    NSCharacterSet *allowed = [NSCharacterSet characterSetWithCharactersInString:@"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-"];
    for (NSString *part in [stem componentsSeparatedByString:@"/"]) {
        if (!part.length || [part rangeOfCharacterFromSet:allowed.invertedSet].location != NSNotFound) return nil;
    }
    return relative;
}
static NSError *LoadError(void) { return [NSError errorWithDomain:@"XzSvg" code:1 userInfo:@{NSLocalizedDescriptionKey:@"SVG source is not a trusted bounded bundled resource"}]; }

@interface XzSvgView ()
@property (nonatomic, strong) NSURL *resourceRoot;
@property (nonatomic, copy) NSString *css_src;
@property (nonatomic, copy) NSString *css_tintColor;
@property (nonatomic, copy) KuiklyRenderCallback css_loadError;
@property (nonatomic, strong) XzSvgDocument *document;
@property (nonatomic, strong) CAShapeLayer *vectorLayer;
@property (nonatomic, strong) UIColor *vectorTint;
@property (nonatomic) CGAffineTransform localTransform;
@property (nonatomic, strong) NSError *loadError;
@end

@implementation XzSvgView
@synthesize hr_rootView;
+ (void)setResourceRootURL:(NSURL *)resourceRootURL {
    NSAssert(NSThread.isMainThread,@"Configure SVG resources on main thread");
    ResourceRoot = resourceRootURL.isFileURL ? resourceRootURL.URLByResolvingSymlinksInPath : nil;
}
- (instancetype)initWithFrame:(CGRect)frame {
    if ((self = [super initWithFrame:frame])) { _resourceRoot = ResourceRoot; self.clipsToBounds = YES; }
    return self;
}
- (void)hrv_setPropWithKey:(NSString *)propKey propValue:(id)propValue {
    KUIKLY_SET_CSS_COMMON_PROP;
}
- (void)setCss_tintColor:(NSString *)value {
    _css_tintColor = [value isKindOfClass:NSString.class] ? [value copy] : nil;
    // This is the pinned SDK converter, not a CSS/hex parser: Kotlin Color -> decimal ARGB.
    self.vectorTint = _css_tintColor.length ? [UIView css_color:_css_tintColor] : nil;
    [CATransaction begin]; [CATransaction setDisableActions:YES];
    [self.document.root applyTint:self.vectorTint toLayer:self.vectorLayer];
    [CATransaction commit];
}
- (void)setCss_loadError:(KuiklyRenderCallback)callback {
    _css_loadError = [callback copy];
    if (callback && self.loadError) callback(@{@"errorCode":@(self.loadError.code)});
}
- (void)setCss_src:(NSString *)value {
    NSString *src = [value isKindOfClass:NSString.class] ? value : nil;
    if ([_css_src isEqualToString:src]) return;
    _css_src = [src copy];
    [self.vectorLayer removeFromSuperlayer]; self.vectorLayer = nil; self.document = nil; self.loadError = nil;
    if (!src.length) return;
    NSString *relative = TrustedRelativePath(src);
    NSError *error = nil;
    if (relative && self.resourceRoot) {
        NSURL *file = [self.resourceRoot URLByAppendingPathComponent:relative];
        // Reject symlink escapes (including aliases to any untrusted local file).
        if ([file.URLByResolvingSymlinksInPath.path isEqualToString:file.path]) {
            NSString *key = [file.path copy];
            self.document = [Documents() objectForKey:key];
            if (!self.document) {
                NSDictionary *attrs = [NSFileManager.defaultManager attributesOfItemAtPath:file.path error:&error];
                if ([attrs[NSFileType] isEqualToString:NSFileTypeRegular] && [attrs[NSFileSize] unsignedLongLongValue] <= 65536) {
                    NSFileHandle *handle = [NSFileHandle fileHandleForReadingFromURL:file error:&error];
                    NSData *data = [handle readDataUpToLength:65537 error:&error];
                    [handle closeFile];
                    if (data) self.document = [XzSvgDocument parseData:data error:&error];
                    if (self.document) [Documents() setObject:self.document forKey:key cost:data.length + 4096];
                }
            }
        }
    }
    if (!self.document) {
        self.loadError = error.domain && [error.domain isEqualToString:@"XzSvg"] ? error : LoadError();
        if (self.css_loadError) self.css_loadError(@{@"errorCode":@(self.loadError.code)});
#if DEBUG
        NSLog(@"[XzSvgView] load failed source=%@ code=%ld",relative ? relative.lastPathComponent : @"<rejected>",(long)self.loadError.code);
#endif
        return;
    }
    self.vectorLayer = [self.document.root makeLayerWithTint:self.vectorTint];
    self.localTransform = self.vectorLayer.affineTransform;
    [self.layer addSublayer:self.vectorLayer];
    [self setNeedsLayout];
    [self layoutVector];
}
- (void)layoutSubviews { [super layoutSubviews]; [self layoutVector]; }
- (void)layoutVector {
    if (!self.document || !self.vectorLayer) return;
    CGRect box = self.document.viewBox, bounds = self.bounds;
    if (!isfinite(bounds.origin.x) || !isfinite(bounds.origin.y) || fabs(bounds.origin.x) > 1000000 || fabs(bounds.origin.y) > 1000000 || !isfinite(bounds.size.width) || !isfinite(bounds.size.height) || bounds.size.width <= 0 || bounds.size.height <= 0 || bounds.size.width > 1000000 || bounds.size.height > 1000000) { self.vectorLayer.hidden = YES; return; }
    self.vectorLayer.hidden = NO;
    CGFloat scale = MIN(bounds.size.width/box.size.width,bounds.size.height/box.size.height);
    CGFloat x = CGRectGetMidX(bounds)-box.size.width*scale/2-box.origin.x*scale;
    CGFloat y = CGRectGetMidY(bounds)-box.size.height*scale/2-box.origin.y*scale;
    if (!isfinite(scale) || !isfinite(x) || !isfinite(y)) { self.vectorLayer.hidden = YES; return; }
    CGAffineTransform fit = CGAffineTransformMake(scale,0,0,scale,x,y);
    [CATransaction begin]; [CATransaction setDisableActions:YES];
    // Root local transform is preserved inside an aspect-fit coordinate system.
    // Compose with fit, never alter/tessellate a path on resize.
    self.vectorLayer.affineTransform = CGAffineTransformConcat(self.localTransform,fit);
    [CATransaction commit];
}
@end
