#import "XzSvgDocument.h"
#import "XzSvgPath.h"
#import <math.h>

static NSError *SvgError(void) { return [NSError errorWithDomain:@"XzSvg" code:2 userInfo:@{NSLocalizedDescriptionKey:@"Unsupported, malformed or over-limit SVG"}]; }
static NSString *Trim(NSString *s) { return [s stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet]; }
static BOOL Scalar(NSString *s, double *n) {
    NSArray *a = XzSvgNumbers(s);
    if (a.count != 1) return NO;
    *n = [a[0] doubleValue]; return YES;
}
static UIColor *Paint(NSString *s) {
    if ([s isEqualToString:@"none"] || [s isEqualToString:@"currentColor"]) return UIColor.clearColor;
    NSDictionary *named = @{@"black":UIColor.blackColor,@"white":UIColor.whiteColor,@"red":UIColor.redColor,
                           @"green":[UIColor colorWithRed:0 green:128.0/255.0 blue:0 alpha:1],@"blue":UIColor.blueColor,@"transparent":UIColor.clearColor};
    if (named[s]) return named[s];
    if (![s hasPrefix:@"#"]) return nil;
    NSString *hex = [s substringFromIndex:1];
    if (hex.length != 3 && hex.length != 4 && hex.length != 6 && hex.length != 8) return nil;
    if ([hex rangeOfCharacterFromSet:[NSCharacterSet characterSetWithCharactersInString:@"0123456789abcdefABCDEF"].invertedSet].location != NSNotFound) return nil;
    unsigned long long value = 0;
    NSScanner *scanner = [NSScanner scannerWithString:hex];
    scanner.charactersToBeSkipped = nil;
    if (![scanner scanHexLongLong:&value] || !scanner.isAtEnd) return nil;
    double r,g,b,a = 1;
    if (hex.length <= 4) {
        if (hex.length == 4) { a = (value & 15)/15.0; value >>= 4; }
        r = ((value >> 8)&15)/15.0; g = ((value >> 4)&15)/15.0; b = (value&15)/15.0;
    } else {
        if (hex.length == 8) { a = (value&255)/255.0; value >>= 8; }
        r = ((value>>16)&255)/255.0; g = ((value>>8)&255)/255.0; b = (value&255)/255.0;
    }
    return [UIColor colorWithRed:r green:g blue:b alpha:a];
}

@interface XzSvgNode ()
@property (nonatomic, readwrite, copy) NSArray<XzSvgNode *> *children;
@property (nonatomic, readwrite) CGPathRef path;
@property (nonatomic, copy) NSDictionary<NSString *, NSString *> *style;
@property (nonatomic) CGAffineTransform transform;
@property (nonatomic) CGAffineTransform cumulativeTransform;
@property (nonatomic) float opacity;
@end

@implementation XzSvgNode
- (void)dealloc { if (_path) CGPathRelease(_path); }
- (UIColor *)colorFor:(NSString *)key tint:(UIColor *)tint {
    NSString *paint = self.style[key];
    if ([paint isEqualToString:@"none"]) return nil;
    UIColor *color = [paint isEqualToString:@"currentColor"] ? (tint ?: Paint(self.style[@"color"])) : Paint(paint);
    CGFloat opacity = [self.style[[key stringByAppendingString:@"-opacity"]] doubleValue];
    return [color colorWithAlphaComponent:CGColorGetAlpha(color.CGColor)*opacity];
}
- (void)applyTint:(UIColor *)tint toLayer:(CAShapeLayer *)layer {
    layer.fillColor = [self colorFor:@"fill" tint:tint].CGColor;
    layer.strokeColor = [self colorFor:@"stroke" tint:tint].CGColor;
    for (NSUInteger i = 0; i < self.children.count; i++) [self.children[i] applyTint:tint toLayer:(CAShapeLayer *)layer.sublayers[i]];
}
- (CAShapeLayer *)makeLayerWithTint:(UIColor *)tint {
    CAShapeLayer *layer = [CAShapeLayer layer];
    layer.anchorPoint = CGPointZero;
    layer.affineTransform = self.transform;
    layer.opacity = self.opacity;
    layer.path = self.path;
    layer.lineWidth = [self.style[@"stroke-width"] doubleValue];
    layer.lineCap = self.style[@"stroke-linecap"];
    layer.lineJoin = self.style[@"stroke-linejoin"];
    layer.fillRule = self.style[@"fill-rule"];
    layer.shouldRasterize = NO;
    layer.actions = @{@"path":NSNull.null,@"transform":NSNull.null,@"fillColor":NSNull.null,@"strokeColor":NSNull.null,@"position":NSNull.null,@"bounds":NSNull.null};
    for (XzSvgNode *child in self.children) [layer addSublayer:[child makeLayerWithTint:tint]];
    [self applyTint:tint toLayer:layer];
    return layer;
}
@end

@interface XzSvgDocument ()
@property (nonatomic, readwrite) CGRect viewBox;
@property (nonatomic, readwrite, strong) XzSvgNode *root;
@end
@interface XzSvgParser : NSObject <NSXMLParserDelegate>
@property (nonatomic, strong) XzSvgDocument *document;
@property (nonatomic, strong) NSMutableArray<XzSvgNode *> *stack;
@property (nonatomic) NSUInteger nodeCount;
@property (nonatomic) NSUInteger pathBytes;
@property (nonatomic) BOOL failed;
@end

static NSSet *StyleKeys(void) {
    return [NSSet setWithArray:@[@"fill",@"stroke",@"color",@"stroke-width",@"stroke-linecap",@"stroke-linejoin",@"fill-opacity",@"stroke-opacity",@"fill-rule"]];
}
static NSDictionary *Defaults(void) {
    return @{@"fill":@"black",@"stroke":@"none",@"color":@"black",@"stroke-width":@"1",@"stroke-linecap":@"butt",@"stroke-linejoin":@"miter",@"fill-opacity":@"1",@"stroke-opacity":@"1",@"fill-rule":@"nonzero"};
}
static BOOL GeometryNumber(NSDictionary *attrs, NSString *key, double fallback, double *out) {
    *out = fallback; return !attrs[key] || Scalar(attrs[key],out);
}
static CGPathRef Shape(NSString *name, NSDictionary *a) CF_RETURNS_RETAINED;
static CGPathRef Shape(NSString *name, NSDictionary *a) {
    if ([name isEqualToString:@"path"]) return XzSvgCreatePath(a[@"d"]);
    CGMutablePathRef p = CGPathCreateMutable();
    BOOL valid = YES;
    double x=0,y=0,w=0,h=0,rx=0,ry=0;
    if ([name isEqualToString:@"rect"]) {
        valid = GeometryNumber(a,@"x",0,&x) && GeometryNumber(a,@"y",0,&y) && GeometryNumber(a,@"width",0,&w) && GeometryNumber(a,@"height",0,&h)
             && GeometryNumber(a,@"rx",0,&rx) && GeometryNumber(a,@"ry",a[@"rx"] ? rx : 0,&ry);
        if (!a[@"rx"] && a[@"ry"]) rx = ry;
        valid = valid && w >= 0 && h >= 0 && rx >= 0 && ry >= 0;
        if (valid && w && h) CGPathAddRoundedRect(p,NULL,CGRectMake(x,y,w,h),fmin(rx,w/2),fmin(ry,h/2));
    } else if ([name isEqualToString:@"circle"] || [name isEqualToString:@"ellipse"]) {
        valid = GeometryNumber(a,@"cx",0,&x) && GeometryNumber(a,@"cy",0,&y);
        if ([name isEqualToString:@"circle"]) { valid = valid && GeometryNumber(a,@"r",0,&rx); ry = rx; }
        else valid = valid && GeometryNumber(a,@"rx",0,&rx) && GeometryNumber(a,@"ry",0,&ry);
        valid = valid && rx >= 0 && ry >= 0;
        if (valid && rx && ry) CGPathAddEllipseInRect(p,NULL,CGRectMake(x-rx,y-ry,rx*2,ry*2));
    } else if ([name isEqualToString:@"line"]) {
        valid = GeometryNumber(a,@"x1",0,&x) && GeometryNumber(a,@"y1",0,&y) && GeometryNumber(a,@"x2",0,&w) && GeometryNumber(a,@"y2",0,&h);
        if (valid) { CGPathMoveToPoint(p,NULL,x,y); CGPathAddLineToPoint(p,NULL,w,h); }
    } else {
        NSArray<NSNumber *> *points = XzSvgNumbers(a[@"points"]);
        valid = points.count >= 4 && points.count % 2 == 0;
        if (valid) {
            CGPathMoveToPoint(p,NULL,points[0].doubleValue,points[1].doubleValue);
            for (NSUInteger i = 2; i < points.count; i += 2) CGPathAddLineToPoint(p,NULL,points[i].doubleValue,points[i+1].doubleValue);
            if ([name isEqualToString:@"polygon"]) CGPathCloseSubpath(p);
        }
    }
    CGPathRef result = valid ? CGPathCreateCopy(p) : nil;
    CGPathRelease(p); return result;
}

@implementation XzSvgParser
- (void)fail:(NSXMLParser *)parser { self.failed = YES; [parser abortParsing]; }
- (void)parser:(NSXMLParser *)parser didStartElement:(NSString *)name namespaceURI:(NSString *)uri qualifiedName:(NSString *)qualified attributes:(NSDictionary<NSString *,NSString *> *)attrs {
    if (++self.nodeCount > 512 || self.stack.count >= 32 || attrs.count > 32) { [self fail:parser]; return; }
    BOOL root = self.nodeCount == 1;
    NSSet *shapes = [NSSet setWithArray:@[@"path",@"rect",@"circle",@"ellipse",@"line",@"polyline",@"polygon"]];
    if ((root && ![name isEqualToString:@"svg"]) || (!root && ![name isEqualToString:@"g"] && ![shapes containsObject:name]) || (!root && !self.stack.count)) { [self fail:parser]; return; }
    XzSvgNode *parent = self.stack.lastObject;
    if (parent.path) { [self fail:parser]; return; }
    NSMutableDictionary *style = [parent.style ?: Defaults() mutableCopy];
    NSMutableDictionary *a = [attrs mutableCopy];
    // Small inline presentation styles only; no CSS selectors, URLs or external resources.
    if (a[@"style"]) {
        for (NSString *part in [a[@"style"] componentsSeparatedByString:@";"]) {
            if (!Trim(part).length) continue;
            NSArray *kv = [part componentsSeparatedByString:@":"];
            if (kv.count != 2 || (![StyleKeys() containsObject:Trim(kv[0])] && ![Trim(kv[0]) isEqualToString:@"opacity"])) { [self fail:parser]; return; }
            a[Trim(kv[0])] = Trim(kv[1]);
        }
        [a removeObjectForKey:@"style"];
    }
    NSDictionary *geometry = @{@"path":@[@"d"],@"rect":@[@"x",@"y",@"width",@"height",@"rx",@"ry"],@"circle":@[@"cx",@"cy",@"r"],@"ellipse":@[@"cx",@"cy",@"rx",@"ry"],@"line":@[@"x1",@"y1",@"x2",@"y2"],@"polyline":@[@"points"],@"polygon":@[@"points"]};
    NSSet *rootKeys = [NSSet setWithArray:@[@"xmlns",@"width",@"height",@"viewBox",@"preserveAspectRatio",@"version"]];
    for (NSString *key in a.allKeys) {
        NSString *value = Trim(a[key]); a[key] = value;
        if ([StyleKeys() containsObject:key]) {
            if ([value isEqualToString:@"inherit"]) continue;
            if ([@[@"fill",@"stroke",@"color"] containsObject:key]) {
                if (!Paint(value) || ([key isEqualToString:@"color"] && [value isEqualToString:@"none"])) { [self fail:parser]; return; }
                if ([key isEqualToString:@"color"] && [value isEqualToString:@"currentColor"]) continue;
            } else if ([@[@"stroke-width",@"fill-opacity",@"stroke-opacity"] containsObject:key]) {
                double n; if (!Scalar(value,&n) || n < 0 || (![key isEqualToString:@"stroke-width"] && n > 1)) { [self fail:parser]; return; }
            } else if ([key isEqualToString:@"stroke-linecap"] && ![@[@"butt",@"round",@"square"] containsObject:value]) { [self fail:parser]; return; }
            else if ([key isEqualToString:@"stroke-linejoin"] && ![@[@"miter",@"round",@"bevel"] containsObject:value]) { [self fail:parser]; return; }
            else if ([key isEqualToString:@"fill-rule"] && ![@[@"nonzero",@"evenodd"] containsObject:value]) { [self fail:parser]; return; }
            style[key] = value;
        } else if (![key isEqualToString:@"opacity"] && ![key isEqualToString:@"transform"] && ![geometry[name] containsObject:key] && !(root && [rootKeys containsObject:key])) { [self fail:parser]; return; }
    }
    XzSvgNode *node = [XzSvgNode new]; node.children = @[]; node.style = style;
    double opacity = 1;
    CGAffineTransform transform;
    if ((a[@"opacity"] && (!Scalar(a[@"opacity"],&opacity) || opacity < 0 || opacity > 1)) || !XzSvgTransform(a[@"transform"],&transform)) { [self fail:parser]; return; }
    node.opacity = (float)opacity; node.transform = transform;
    node.cumulativeTransform = CGAffineTransformConcat(transform,parent ? parent.cumulativeTransform : CGAffineTransformIdentity);
    CGAffineTransform combined = node.cumulativeTransform;
    const CGFloat parts[] = {combined.a,combined.b,combined.c,combined.d,combined.tx,combined.ty};
    for (NSUInteger i = 0; i < 6; i++) if (!isfinite(parts[i]) || fabs(parts[i]) > 1000000) { [self fail:parser]; return; }
    if (root) {
        if ((a[@"xmlns"] && ![a[@"xmlns"] isEqualToString:@"http://www.w3.org/2000/svg"]) || (a[@"preserveAspectRatio"] && ![@[@"xMidYMid",@"xMidYMid meet"] containsObject:a[@"preserveAspectRatio"]])) { [self fail:parser]; return; }
        double width=0,height=0;
        if (!GeometryNumber(a,@"width",0,&width) || !GeometryNumber(a,@"height",0,&height) || width < 0 || height < 0) { [self fail:parser]; return; }
        NSArray<NSNumber *> *box = a[@"viewBox"] ? XzSvgNumbers(a[@"viewBox"]) : @[@0,@0,@(width),@(height)];
        if (box.count != 4 || box[2].doubleValue < 0.000001 || box[3].doubleValue < 0.000001) { [self fail:parser]; return; }
        self.document.viewBox = CGRectMake(box[0].doubleValue,box[1].doubleValue,box[2].doubleValue,box[3].doubleValue);
        self.document.root = node;
    } else if ([shapes containsObject:name]) {
        self.pathBytes += [a[@"d"] length] + [a[@"points"] length];
        if (self.pathBytes > 65536) { [self fail:parser]; return; }
        node.path = Shape(name,a);
        if (!node.path) { [self fail:parser]; return; }
        if (!CGPathIsEmpty(node.path)) {
            CGRect bounds = CGRectApplyAffineTransform(CGPathGetBoundingBox(node.path),combined);
            const CGFloat edges[] = {CGRectGetMinX(bounds),CGRectGetMinY(bounds),CGRectGetMaxX(bounds),CGRectGetMaxY(bounds)};
            for (NSUInteger i = 0; i < 4; i++) if (!isfinite(edges[i]) || fabs(edges[i]) > 1000000) { [self fail:parser]; return; }
        }
    }
    if (parent) parent.children = [parent.children arrayByAddingObject:node];
    [self.stack addObject:node];
}
- (void)parser:(NSXMLParser *)parser didEndElement:(NSString *)name namespaceURI:(NSString *)uri qualifiedName:(NSString *)qualified { [self.stack removeLastObject]; }
- (void)parser:(NSXMLParser *)parser foundCharacters:(NSString *)string { if (Trim(string).length) [self fail:parser]; }
- (void)parser:(NSXMLParser *)parser foundCDATA:(NSData *)block { [self fail:parser]; }
- (void)parser:(NSXMLParser *)parser foundProcessingInstructionWithTarget:(NSString *)target data:(NSString *)data { [self fail:parser]; }
- (void)parser:(NSXMLParser *)parser foundInternalEntityDeclarationWithName:(NSString *)name value:(NSString *)value { [self fail:parser]; }
- (void)parser:(NSXMLParser *)parser foundExternalEntityDeclarationWithName:(NSString *)name publicID:(NSString *)publicID systemID:(NSString *)systemID { [self fail:parser]; }
- (NSData *)parser:(NSXMLParser *)parser resolveExternalEntityName:(NSString *)name systemID:(NSString *)systemID { [self fail:parser]; return nil; }
@end

@implementation XzSvgDocument
+ (instancetype)parseData:(NSData *)data error:(NSError **)error {
    if (error) *error = nil;
    if (!data.length || data.length > 65536) { if (error) *error = SvgError(); return nil; }
    NSString *text = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    // Reject declarations before NSXMLParser can interpret DTD/entity expansion. Comments are fine.
    if (!text || [text rangeOfString:@"<!DOCTYPE" options:NSCaseInsensitiveSearch].location != NSNotFound || [text rangeOfString:@"<!ENTITY" options:NSCaseInsensitiveSearch].location != NSNotFound || [text rangeOfString:@"\0"].location != NSNotFound) {
        if (error) *error = SvgError(); return nil;
    }
    XzSvgParser *delegate = [XzSvgParser new]; delegate.document = [self new]; delegate.stack = [NSMutableArray array];
    NSXMLParser *parser = [[NSXMLParser alloc] initWithData:data];
    parser.shouldResolveExternalEntities = NO;
    parser.externalEntityResolvingPolicy = NSXMLParserResolveExternalEntitiesNever;
    parser.delegate = delegate;
    if (![parser parse] || delegate.failed || !delegate.document.root || delegate.stack.count) { if (error) *error = SvgError(); return nil; }
    return delegate.document;
}
@end
