#import "XzSvgPath.h"
#import <math.h>
#import <ctype.h>

static void Space(const char **p) { while (**p == ' ' || **p == '\t' || **p == '\n' || **p == '\r') (*p)++; }
static void Separator(const char **p) { Space(p); if (**p == ',') { (*p)++; Space(p); } }

BOOL XzSvgReadNumber(const char **cursor, double *value) {
    const char *p = *cursor;
    Space(&p);
    const char *start = p;
    if (*p == '+' || *p == '-') p++;
    NSUInteger digits = 0;
    while (isdigit((unsigned char)*p)) { p++; digits++; }
    if (*p == '.') { p++; while (isdigit((unsigned char)*p)) { p++; digits++; } }
    if (!digits) return NO;
    if (*p == 'e' || *p == 'E') {
        p++; if (*p == '+' || *p == '-') p++;
        if (!isdigit((unsigned char)*p)) return NO;
        while (isdigit((unsigned char)*p)) p++;
    }
    if (p - start > 64) return NO;
    char buffer[65] = {0}; memcpy(buffer, start, (size_t)(p - start));
    double n = strtod(buffer, NULL);
    if (!isfinite(n) || fabs(n) > 1000000) return NO;
    *cursor = p; *value = n;
    return YES;
}

NSArray<NSNumber *> *XzSvgNumbers(NSString *text) {
    if (!text || text.length > 32768) return nil;
    const char *p = text.UTF8String;
    NSMutableArray *out = [NSMutableArray array];
    Space(&p);
    while (*p) {
        double n;
        if (!XzSvgReadNumber(&p, &n) || out.count >= 8192) return nil;
        [out addObject:@(n)];
        Space(&p);
        if (*p == ',') { p++; Space(&p); if (!*p || *p == ',') return nil; }
    }
    return out;
}

BOOL XzSvgTransform(NSString *text, CGAffineTransform *result) {
    *result = CGAffineTransformIdentity;
    if (!text) return YES;
    if (text.length > 4096) return NO;
    const char *p = text.UTF8String;
    NSUInteger count = 0;
    Space(&p);
    while (*p) {
        if (++count > 32) return NO;
        const char *start = p;
        while (isalpha((unsigned char)*p)) p++;
        NSString *name = [[NSString alloc] initWithBytes:start length:(NSUInteger)(p-start) encoding:NSUTF8StringEncoding];
        Space(&p); if (*p++ != '(') return NO;
        start = p;
        while (*p && *p != ')') p++;
        if (*p != ')') return NO;
        NSString *args = [[NSString alloc] initWithBytes:start length:(NSUInteger)(p-start) encoding:NSUTF8StringEncoding];
        NSArray<NSNumber *> *a = XzSvgNumbers(args);
        if (!a) return NO;
        CGAffineTransform t = CGAffineTransformIdentity;
        double x = a.count ? a[0].doubleValue : 0, y = a.count > 1 ? a[1].doubleValue : 0;
        if ([name isEqualToString:@"matrix"] && a.count == 6) {
            t = CGAffineTransformMake(x, y, a[2].doubleValue, a[3].doubleValue, a[4].doubleValue, a[5].doubleValue);
        } else if ([name isEqualToString:@"translate"] && (a.count == 1 || a.count == 2)) {
            t = CGAffineTransformMakeTranslation(x, y);
        } else if ([name isEqualToString:@"scale"] && (a.count == 1 || a.count == 2)) {
            t = CGAffineTransformMakeScale(x, a.count == 1 ? x : y);
        } else if ([name isEqualToString:@"rotate"] && (a.count == 1 || a.count == 3)) {
            double cy = a.count == 3 ? a[2].doubleValue : 0;
            t = CGAffineTransformMakeTranslation(a.count == 3 ? y : 0, cy);
            t = CGAffineTransformRotate(t, x * M_PI / 180);
            t = CGAffineTransformTranslate(t, a.count == 3 ? -y : 0, -cy);
        } else if (([name isEqualToString:@"skewX"] || [name isEqualToString:@"skewY"]) && a.count == 1) {
            double skew = tan(x * M_PI / 180);
            t = [name isEqualToString:@"skewX"] ? CGAffineTransformMake(1,0,skew,1,0,0) : CGAffineTransformMake(1,skew,0,1,0,0);
        } else return NO;
        // SVG transforms are post-multiplied: rightmost transform acts first.
        *result = CGAffineTransformConcat(t, *result);
        const CGFloat parts[] = {result->a,result->b,result->c,result->d,result->tx,result->ty};
        for (NSUInteger i = 0; i < 6; i++) if (!isfinite(parts[i]) || fabs(parts[i]) > 1000000) return NO;
        p++; Space(&p); if (*p == ',') { p++; Space(&p); if (!*p) return NO; }
    }
    return YES;
}

static CGPoint SvgPoint(double x, double y, CGPoint base, BOOL relative) {
    return CGPointMake(x + (relative ? base.x : 0), y + (relative ? base.y : 0));
}
static BOOL SafeSvgPoint(CGPoint p) { return isfinite(p.x) && isfinite(p.y) && fabs(p.x) <= 1000000 && fabs(p.y) <= 1000000; }
static CGPoint Reflect(CGPoint control, CGPoint end) { return CGPointMake(2*end.x-control.x, 2*end.y-control.y); }

// SVG endpoint arc -> cubic Beziers, each spanning <= 90 degrees; no sampling/bitmap.
static BOOL Arc(CGMutablePathRef path, CGPoint from, CGPoint to, double rx, double ry,
                double degrees, BOOL large, BOOL sweep) {
    rx = fabs(rx); ry = fabs(ry);
    if (CGPointEqualToPoint(from, to)) return YES;
    if (rx == 0 || ry == 0) { CGPathAddLineToPoint(path, NULL, to.x, to.y); return YES; }
    double angle = fmod(degrees, 360) * M_PI / 180, c = cos(angle), s = sin(angle);
    double dx = (from.x-to.x)/2, dy = (from.y-to.y)/2;
    double x = c*dx+s*dy, y = -s*dx+c*dy;
    double lambda = x*x/(rx*rx)+y*y/(ry*ry);
    if (lambda > 1) { rx *= sqrt(lambda); ry *= sqrt(lambda); }
    if (!isfinite(lambda) || !isfinite(rx) || !isfinite(ry)) return NO;
    double numerator = fmax(0, rx*rx*ry*ry-rx*rx*y*y-ry*ry*x*x);
    double denominator = rx*rx*y*y+ry*ry*x*x;
    if (!isfinite(numerator) || !isfinite(denominator) || denominator <= 0) return NO;
    double factor = (large == sweep ? -1 : 1)*sqrt(numerator/denominator);
    double cx = factor*rx*y/ry, cy = -factor*ry*x/rx;
    double centerX = c*cx-s*cy+(from.x+to.x)/2, centerY = s*cx+c*cy+(from.y+to.y)/2;
    double ux = (x-cx)/rx, uy = (y-cy)/ry, vx = (-x-cx)/rx, vy = (-y-cy)/ry;
    double start = atan2(uy,ux), delta = atan2(ux*vy-uy*vx,ux*vx+uy*vy);
    if (!sweep && delta > 0) delta -= 2*M_PI;
    if (sweep && delta < 0) delta += 2*M_PI;
    if (!isfinite(start) || !isfinite(delta)) return NO;
    int segments = (int)ceil(fabs(delta)/(M_PI/2));
    if (segments < 1 || segments > 4 || !isfinite(centerX) || !isfinite(centerY)) return NO;
    CGAffineTransform t = CGAffineTransformMake(c*rx,s*rx,-s*ry,c*ry,centerX,centerY);
    for (int i = 0; i < segments; i++) {
        double a = start + delta*i/segments, b = start + delta*(i+1)/segments;
        double k = 4.0/3.0*tan((b-a)/4);
        CGPoint p1 = CGPointApplyAffineTransform(CGPointMake(cos(a)-k*sin(a),sin(a)+k*cos(a)),t);
        CGPoint p2 = CGPointApplyAffineTransform(CGPointMake(cos(b)+k*sin(b),sin(b)-k*cos(b)),t);
        CGPoint p3 = i == segments-1 ? to : CGPointApplyAffineTransform(CGPointMake(cos(b),sin(b)),t);
        if (!SafeSvgPoint(p1) || !SafeSvgPoint(p2) || !SafeSvgPoint(p3)) return NO;
        CGPathAddCurveToPoint(path,NULL,p1.x,p1.y,p2.x,p2.y,p3.x,p3.y);
    }
    return YES;
}

CGPathRef XzSvgCreatePath(NSString *text) {
    if (!text.length || text.length > 32768) return nil;
    const char *p = text.UTF8String;
    CGMutablePathRef path = CGPathCreateMutable();
    CGPoint current = CGPointZero, origin = CGPointZero, cubic = CGPointZero, quad = CGPointZero;
    char command = 0, previous = 0;
    NSUInteger operations = 0;
    BOOL valid = YES, moved = NO;
    Space(&p);
    while (*p && valid) {
        if (++operations > 8192) { valid = NO; break; }
        if (isalpha((unsigned char)*p)) command = *p++;
        char upper = (char)toupper(command);
        BOOL relative = islower(command);
        if (!moved && upper != 'M') { valid = NO; break; }
        if (upper == 'Z') {
            CGPathCloseSubpath(path); current = origin; previous = 'Z'; command = 0;
            Space(&p); continue;
        }
        int count = 0;
        switch (upper) {
            case 'M': case 'L': case 'T': count = 2; break;
            case 'H': case 'V': count = 1; break;
            case 'C': count = 6; break;
            case 'S': case 'Q': count = 4; break;
            case 'A': count = 7; break;
            default: valid = NO;
        }
        double a[7] = {0};
        for (int i = 0; valid && i < count; i++) {
            if (i) Separator(&p);
            // Arc flags are single grammar tokens, including adjacent "01" flags.
            if (upper == 'A' && (i == 3 || i == 4)) {
                Space(&p); if (*p != '0' && *p != '1') valid = NO; else a[i] = *p++-'0';
            } else if (!XzSvgReadNumber(&p, &a[i])) valid = NO;
        }
        if (!valid) break;
        CGPoint end = current, c1 = current, c2 = current;
        switch (upper) {
            case 'M': case 'L': case 'T': end = SvgPoint(a[0],a[1],current,relative); break;
            case 'H': end.x = a[0]+(relative ? current.x : 0); break;
            case 'V': end.y = a[0]+(relative ? current.y : 0); break;
            case 'C': c1 = SvgPoint(a[0],a[1],current,relative); c2 = SvgPoint(a[2],a[3],current,relative); end = SvgPoint(a[4],a[5],current,relative); break;
            case 'S': c1 = previous == 'C' || previous == 'S' ? Reflect(cubic,current) : current;
                c2 = SvgPoint(a[0],a[1],current,relative); end = SvgPoint(a[2],a[3],current,relative); break;
            case 'Q': c1 = SvgPoint(a[0],a[1],current,relative); end = SvgPoint(a[2],a[3],current,relative); break;
            case 'A': end = SvgPoint(a[5],a[6],current,relative); break;
        }
        if (upper == 'T') c1 = previous == 'Q' || previous == 'T' ? Reflect(quad,current) : current;
        if (!SafeSvgPoint(end) || !SafeSvgPoint(c1) || !SafeSvgPoint(c2)) { valid = NO; break; }
        switch (upper) {
            case 'M': CGPathMoveToPoint(path,NULL,end.x,end.y); origin = end; moved = YES; command = relative ? 'l' : 'L'; break;
            case 'L': case 'H': case 'V': CGPathAddLineToPoint(path,NULL,end.x,end.y); break;
            case 'C': case 'S': CGPathAddCurveToPoint(path,NULL,c1.x,c1.y,c2.x,c2.y,end.x,end.y); cubic = c2; break;
            case 'Q': case 'T': CGPathAddQuadCurveToPoint(path,NULL,c1.x,c1.y,end.x,end.y); quad = c1; break;
            case 'A': if (a[0] < 0 || a[1] < 0 || !Arc(path,current,end,a[0],a[1],a[2],a[3] != 0,a[4] != 0)) valid = NO; break;
        }
        previous = upper; current = end;
        Space(&p);
        if (*p == ',') { p++; Space(&p); if (!*p || isalpha((unsigned char)*p)) valid = NO; }
    }
    CGPathRef result = valid && moved ? CGPathCreateCopy(path) : nil;
    CGPathRelease(path);
    return result;
}
