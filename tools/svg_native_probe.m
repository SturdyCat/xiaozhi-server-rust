#import <UIKit/UIKit.h>
#import <QuartzCore/QuartzCore.h>
#import "XzSvgView.h"
#import "XzSvgDocument.h"
#import "XzSvgPath.h"
#import "KuiklyRenderLayerHandler.h"
#import "KuiklyRenderBridge.h"

static NSUInteger assertions;
static void Require(BOOL value, NSString *label) {
    assertions++;
    if (!value) { fprintf(stderr,"FAIL %s\n",label.UTF8String); exit(1); }
}
static XzSvgDocument *Parse(NSString *text) { return [XzSvgDocument parseData:[text dataUsingEncoding:NSUTF8StringEncoding] error:nil]; }
static NSString *Wrap(NSString *body) { return [NSString stringWithFormat:@"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'>%@</svg>",body]; }
static NSUInteger CheckLayers(CALayer *layer, NSMutableArray *paths) {
    Require(!layer.contents && !layer.shouldRasterize,@"no bitmap contents/rasterization");
    Require([layer isKindOfClass:CAShapeLayer.class],@"all vector nodes CAShapeLayer");
    NSUInteger n = 0;
    CAShapeLayer *shape = (CAShapeLayer *)layer;
    if (shape.path) { Require(!CGPathIsEmpty(shape.path),@"nonempty path"); n++; if (paths) [paths addObject:[NSValue valueWithPointer:shape.path]]; }
    for (CALayer *child in layer.sublayers) n += CheckLayers(child,paths);
    return n;
}
static BOOL SameColor(CGColorRef a, UIColor *b) {
    if (!a) return NO;
    CGFloat ar=0,ag=0,ab=0,aa=0,br=0,bg=0,bb=0,ba=0;
    if (![[UIColor colorWithCGColor:a] getRed:&ar green:&ag blue:&ab alpha:&aa] || ![b getRed:&br green:&bg blue:&bb alpha:&ba]) return NO;
    return fabs(ar-br)<1e-6 && fabs(ag-bg)<1e-6 && fabs(ab-bb)<1e-6 && fabs(aa-ba)<1e-6;
}
static void ValidPath(NSString *text) { CGPathRef p = XzSvgCreatePath(text); Require(p != nil,[ @"valid path " stringByAppendingString:text]); if (p) CGPathRelease(p); }
static void BadPath(NSString *text) { CGPathRef p = XzSvgCreatePath(text); Require(p == nil,[ @"bad path " stringByAppendingString:text]); if (p) CGPathRelease(p); }

int main(int argc, char **argv) {
    @autoreleasepool {
        Require(argc == 3,@"resource-root and probe-fixture-root arguments");
        NSURL *root = [NSURL fileURLWithPath:[NSString stringWithUTF8String:argv[1]] isDirectory:YES];
        NSURL *fixtures = [NSURL fileURLWithPath:[NSString stringWithUTF8String:argv[2]] isDirectory:YES];
        id previous = [KuiklyRenderBridge componentExpandHandler];
        [XzSvgView setResourceRootURL:root];
        Require([KuiklyRenderBridge componentExpandHandler] == previous,@"existing handler untouched");
        NSArray *names = @[@"save",@"delete",@"menu",@"chevron-down",@"chevron-up",@"chevron-right",@"play",@"stop",@"refresh",@"check",@"close",@"settings",@"microphone",@"waveform",@"text",@"intent",@"model",@"speaker",@"info"];
        UIView *host = [UIView new];
        KuiklyRenderLayerHandler *factory = [[KuiklyRenderLayerHandler alloc] initWithRootView:host contextParam:[KuiklyContextParam new]];
        NSUInteger shapes = 0;
        for (NSString *name in names) {
            [factory createRenderViewWithTag:@1 viewName:@"XzSvgView"];
            XzSvgView *view = (XzSvgView *)[factory viewWithTag:@1];
            Require([view isMemberOfClass:XzSvgView.class],@"real SDK factory XzSvgView");
            [factory setPropWithTag:@1 propKey:@"src" propValue:[NSString stringWithFormat:@"assets://common/icons/source/%@.svg",name]];
            [factory setPropWithTag:@1 propKey:@"tintColor" propValue:@"4294901760"];
            Require([view valueForKey:@"document"] != nil,[ @"bundled parse " stringByAppendingString:name]);
            Require(view.subviews.count == 0 && ![view isKindOfClass:UIImageView.class],@"no image views");
            CAShapeLayer *vector = [view valueForKey:@"vectorLayer"];
            NSMutableArray *before = [NSMutableArray array];
            shapes += CheckLayers(vector,before);
            for (NSNumber *size in @[@24,@48,@128,@500]) {
                [factory setRenderViewFrameWithTag:@1 frame:CGRectMake(0,0,size.doubleValue,size.doubleValue)];
                [view layoutIfNeeded];
                Require(fabs(vector.affineTransform.a-size.doubleValue/24)<1e-8,@"aspect-fit layer transform");
                NSMutableArray *after = [NSMutableArray array]; CheckLayers(vector,after);
                Require([before isEqual:after],@"same paths reused when scaling");
            }
            XzSvgView *other = [XzSvgView new];
            [other hrv_setPropWithKey:@"src" propValue:[NSString stringWithFormat:@"assets://common/icons/source/%@.svg",name]];
            Require([other valueForKey:@"document"] == [view valueForKey:@"document"],@"cache document reuse");
            Require([other valueForKey:@"vectorLayer"] != vector,@"no shared layers");
            [factory removeRenderViewWithTag:@1];
        }
        ValidPath(@"M1 2L3 4H8V9C1 2 3 4 5 6S7 8 9 10Q11 12 13 14T15 16A2 3 45 0 1 20 20Z");
        ValidPath(@"m1 2 3 4h2v-1c1 2 3 4 5 6s7 8 9 10q1 2 3 4t5 6a2 3 45 01 8 8z");
        ValidPath(@"M.5-.5L1e1 2E-1M0 0a0 2 0 0 0 4 4");
        for (NSString *s in @[@"",@"L1 2",@"M0",@"M0 0R1 2",@"M0 0L",@"M0 0,",@"M0 0Z1 2",@"M0 0LNaN 0",@"M0 0L1e100 0",@"M0 0A-1 2 0 0 0 3 3",@"M0 0A1 2 0 2 0 3 3",@"M0 0A1 2 0 0 0 3",@"M0 0,,1 2"]) BadPath(s);
        XzSvgDocument *doc = Parse(Wrap(@"<g opacity='.5' transform='translate(1 2) scale(2)'><path fill='#00ff00' stroke='currentColor' stroke-width='2' fill-opacity='.3' stroke-opacity='.4' d='M0 0L10 10'/><rect x='1' y='2' width='4' height='6' rx='1' ry='2'/><circle cx='3' cy='3' r='2'/><ellipse cx='4' cy='4' rx='2' ry='1'/><line x1='0' y1='0' x2='2' y2='2'/><polyline points='1 1 2 2 3 1'/><polygon points='1 1 2 2 3 1'/></g>"));
        Require(doc != nil,@"all supported primitives/group transform");
        CGAffineTransform skew;
        Require(XzSvgTransform(@"skewX(45) skewY(45)",&skew) && fabs(skew.a-2)<1e-8 && fabs(skew.b-1)<1e-8 && fabs(skew.c-1)<1e-8,@"standard skew transform composition");
        Require(XzSvgTransform(@"rotate(90 5 5)",&skew) && fabs(skew.tx-10)<1e-8 && fabs(skew.ty)<1e-8,@"rotation around center");
        CAShapeLayer *layer = [doc.root makeLayerWithTint:UIColor.redColor];
        Require(CheckLayers(layer,nil) == 7,@"seven vector primitives");
        CAShapeLayer *group = (CAShapeLayer *)layer.sublayers[0], *mixed = (CAShapeLayer *)group.sublayers[0];
        Require(fabs(group.opacity-.5)<1e-6 && fabs(group.affineTransform.a-2)<1e-8 && fabs(group.affineTransform.tx-1)<1e-8,@"group compositing and transform order");
        Require(SameColor(mixed.fillColor,[UIColor colorWithRed:0 green:1 blue:0 alpha:.3]),@"literal fill preserved under tint");
        Require(SameColor(mixed.strokeColor,[UIColor colorWithRed:1 green:0 blue:0 alpha:.4]),@"currentColor uses tint");
        [doc.root applyTint:UIColor.blueColor toLayer:layer];
        Require(SameColor(mixed.fillColor,[UIColor colorWithRed:0 green:1 blue:0 alpha:.3]) && SameColor(mixed.strokeColor,[UIColor colorWithRed:0 green:0 blue:1 alpha:.4]),@"reactive currentColor only tint");
        [doc.root applyTint:nil toLayer:layer];
        Require(SameColor(mixed.strokeColor,[UIColor colorWithRed:0 green:0 blue:0 alpha:.4]),@"tint reset restores default source color");
        XzSvgDocument *sourceColor = Parse(Wrap(@"<g color='red'><path fill='#00ff00' stroke='currentColor' d='M0 0L1 1'/></g>"));
        CAShapeLayer *sourceLayer = [sourceColor.root makeLayerWithTint:UIColor.blueColor];
        CAShapeLayer *sourcePath = (CAShapeLayer *)sourceLayer.sublayers[0].sublayers[0];
        Require(SameColor(sourcePath.strokeColor,UIColor.blueColor),@"provided tint overrides source currentColor");
        [sourceColor.root applyTint:nil toLayer:sourceLayer];
        Require(SameColor(sourcePath.strokeColor,UIColor.redColor) && SameColor(sourcePath.fillColor,UIColor.greenColor),@"tint reset restores source color inheritance and literal fill");
        CGPathRef arc = XzSvgCreatePath(@"M0 0A10 10 0 0 1 20 0");
        CGRect arcBounds = CGPathGetPathBoundingBox(arc);
        Require(fabs(arcBounds.origin.x)<1e-8 && fabs(arcBounds.origin.y+10)<1e-8 && fabs(arcBounds.size.width-20)<1e-8 && fabs(arcBounds.size.height-10)<1e-8,@"arc geometric half-circle bounds");
        CGPathRelease(arc);
        BadPath(@"M0 0A1e-300 1e-300 0 0 1 1 1");
        Require(Parse(Wrap(@"<path style='fill:currentColor;stroke:none;opacity:.2' d='M0 0L1 1'/>") ) != nil,@"whitelisted presentation style");
        for (NSString *s in @[@"<image/>",@"<use/>",@"<defs/>",@"<linearGradient/>",@"<clipPath/>",@"<mask/>",@"<foreignObject/>",@"<script/>",@"<path fill='url(#x)' d='M0 0L1 1'/>",@"<path onclick='x' d='M0 0L1 1'/>",@"<path style='filter:blur(1px)' d='M0 0L1 1'/>",@"<path d='M0 0L1 1' transform='skewX(90)'/>",@"<circle r='-1'/>",@"<rect width='5px'/>",@"<g opacity='2'/>",@"<svg/>",@"<g>not SVG text</g>",@"<![CDATA[x]]>"]) Require(Parse(Wrap(s)) == nil,@"unsupported SVG fails closed");
        for (NSString *s in @[@"<svg viewBox='0 0 0 24'/>",@"<svg viewBox='0 0 24 NaN'/>",@"<svg viewBox='0 0 24 24'>",@"<!DOCTYPE svg [<!ENTITY x 'hi'>]><svg viewBox='0 0 24 24'>&x;</svg>",@"<!DOCTYPE svg SYSTEM 'file:///etc/passwd'><svg viewBox='0 0 24 24'/>"]) Require(Parse(s) == nil,@"malformed/DTD/external entity rejection");
        NSMutableString *deep = [NSMutableString string]; for (int i=0;i<33;i++) [deep appendString:@"<g>"]; for (int i=0;i<33;i++) [deep appendString:@"</g>"];
        Require(Parse(Wrap(deep)) == nil,@"depth bound");
        NSMutableString *many = [NSMutableString string]; for (int i=0;i<513;i++) [many appendString:@"<g/>"];
        Require(Parse(Wrap(many)) == nil,@"node bound");
        Require(Parse([@" " stringByPaddingToLength:65537 withString:@" " startingAtIndex:0]) == nil,@"byte bound");
        Require(Parse(Wrap(@"<path d='M1000001 0L0 0'/>") ) == nil,@"numeric bound");
        Require(Parse(@"<svg viewBox='0 0 1e-300 24'/>") == nil,@"tiny viewBox bound");
        Require(Parse(Wrap(@"<g transform='scale(1000000)'><g transform='scale(1000000)'/></g>")) == nil,@"cumulative transform bound");
        Require(Parse(Wrap(@"<g transform='scale(1000000)'><circle r='5'/></g>")) == nil,@"transformed geometry bound");
        Require(Parse(Wrap(@"<path d='M0 0L1 1' fill='#0x1234'/>")) == nil,@"strict hexadecimal paint grammar");
        NSString *oversizedPath = [@"M0 0" stringByPaddingToLength:32800 withString:@"L1 1 " startingAtIndex:0];
        Require(Parse(Wrap([NSString stringWithFormat:@"<path d='%@'/>",oversizedPath])) == nil,@"path character bound");
        NSString *tooManyOperations = [@"M0 0" stringByPaddingToLength:24580 withString:@"h0 " startingAtIndex:0];
        BadPath(tooManyOperations);
        NSString *tooManyTransforms = [@"" stringByPaddingToLength:330 withString:@"scale(1)  " startingAtIndex:0];
        Require(Parse(Wrap([NSString stringWithFormat:@"<g transform='%@'/>",tooManyTransforms])) == nil,@"transform operation bound");
        NSMutableString *tooManyAttrs = [NSMutableString stringWithString:@"<g"];
        for (int i=0;i<33;i++) [tooManyAttrs appendFormat:@" a%d='0'",i]; [tooManyAttrs appendString:@"/>"];
        Require(Parse(Wrap(tooManyAttrs)) == nil,@"attribute bound");
        [XzSvgView setResourceRootURL:fixtures];
        NSArray *excluded = @[@"https://example.invalid/a.svg",@"file:///tmp/a.svg",@"assets://common/icons/../save.svg",@"assets://common/icons/%73ave.svg",@"assets://common/icons/save.svg?x=1",@"assets://common/icons/save.svg#x",@"assets://other/save.svg",@"assets://common//save.svg",@"assets://common/icons/save.png",@"assets://common/icons/bad.xml.svg",@"assets://common/icons/unknown.svg",@"assets://common/icons/escape.svg",@"assets://common/icons/oversize.svg",@"assets://common/icons/malformed.svg"];
        for (NSString *src in excluded) {
            XzSvgView *v = [XzSvgView new]; __block NSUInteger errors=0;
            [v hrv_setPropWithKey:@"loadError" propValue:^(NSDictionary *e) { Require(e.count == 1 && e[@"errorCode"] != nil,@"error event sanitized"); errors++; }];
            [v hrv_setPropWithKey:@"src" propValue:src];
            Require(![v valueForKey:@"document"] && !v.layer.sublayers.count && errors == 1,@"excluded source clears and fails explicitly");
        }
        XzSvgView *generic = [XzSvgView new];
        [generic hrv_setPropWithKey:@"src" propValue:@"assets://common/icons/new-vector.svg"];
        Require([generic valueForKey:@"document"] != nil,@"generic new bundled SVG without manual icon allowlist");
        [generic hrv_setPropWithKey:@"tintColor" propValue:@"4294901760"];
        CAShapeLayer *current = (CAShapeLayer *)((CAShapeLayer *)[generic valueForKey:@"vectorLayer"]).sublayers[0];
        Require(SameColor(current.strokeColor,UIColor.redColor),@"actual SDK decimal ARGB converter");
        [generic hrv_setPropWithKey:@"tintColor" propValue:@"4278190335"];
        Require(SameColor(current.strokeColor,UIColor.blueColor),@"actual SDK reactive tint update");
        [generic hrv_setPropWithKey:@"tintColor" propValue:@""];
        Require(SameColor(current.strokeColor,UIColor.redColor),@"empty SDK tint reset restores source color");
        generic.frame = CGRectZero; [generic layoutIfNeeded];
        Require(((CALayer *)[generic valueForKey:@"vectorLayer"]).hidden,@"empty layout hides safely without path mutation");
        generic.frame = CGRectMake(0,0,100,50); [generic layoutIfNeeded];
        CAShapeLayer *fit = [generic valueForKey:@"vectorLayer"];
        Require(fabs(fit.affineTransform.a-50.0/24)<1e-8 && fabs(fit.affineTransform.tx-25)<1e-8,@"non-square aspect-fit centered");
        [generic hrv_setPropWithKey:@"src" propValue:@""];
        Require(!generic.layer.sublayers.count,@"empty source clears prior vectors");
        printf("PASS actual macabi UIKit + pinned Kuikly archive: %lu assertions, 19 assets, %lu shapes, 24/48/128/500 scaling, factory/props/tint/cache, Tiny SVG primitives/path/transform, malformed/limits/resource exclusions\n",(unsigned long)assertions,(unsigned long)shapes);
    }
    return 0;
}
