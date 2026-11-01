#import <UIKit/UIKit.h>
#import <OpenKuiklyIOSRender/KuiklyRenderViewExportProtocol.h>

/** Input-only UIKit view. All screen rendering lives in shared Kuikly Compose. */
@interface EchoMuseConsoleKeyboardHost : UIView <KuiklyRenderViewExportProtocol, UIKeyInput, UIGestureRecognizerDelegate>
@property (nonatomic, copy) KuiklyRenderCallback inputCallback;
@property (nonatomic) BOOL inputEnabled;
@property (nonatomic) BOOL disposed;
@property (nonatomic, strong) UITapGestureRecognizer *outsideObserver;
@end

@implementation EchoMuseConsoleKeyboardHost
- (BOOL)canBecomeFirstResponder { return self.inputEnabled && !self.disposed; }
- (BOOL)becomeFirstResponder {
    BOOL ok = [super becomeFirstResponder];
    if (ok && !self.disposed && self.inputCallback) self.inputCallback(@{@"kind": @"focus", @"focused": @YES});
    return ok;
}
- (BOOL)resignFirstResponder {
    BOOL wasFocused = self.isFirstResponder;
    BOOL ok = [super resignFirstResponder];
    if (ok && wasFocused && self.inputCallback) self.inputCallback(@{@"kind": @"focus", @"focused": @NO});
    return ok;
}
- (BOOL)hasText { return YES; }
- (void)insertText:(NSString *)text {
    [self emit:@{@"kind": @"text", @"data": text ?: @""}];
}
- (void)deleteBackward {
    [self emit:@{@"kind": @"key", @"key": @"Backspace"}];
}
- (void)paste:(id)sender {
    NSString *text = UIPasteboard.generalPasteboard.string;
    if (text) [self emit:@{@"kind": @"paste", @"data": text}];
}
- (BOOL)canPerformAction:(SEL)action withSender:(id)sender {
    return action == @selector(paste:) && self.inputEnabled;
}
- (void)emit:(NSDictionary *)event {
    // Export callbacks are wrapped by Kuikly's performOnContextQueue, never call Kotlin directly.
    if (!self.disposed && self.inputEnabled && self.inputCallback) self.inputCallback(event);
}
- (void)hrv_setPropWithKey:(NSString *)propKey propValue:(id)propValue {
    KUIKLY_SET_CSS_COMMON_PROP;
    if ([propKey isEqualToString:@"onInput"]) self.inputCallback = propValue;
    else if ([propKey isEqualToString:@"enabled"]) {
        BOOL enabled = [propValue boolValue];
        if (enabled == self.inputEnabled) return;
        self.inputEnabled = enabled;
        if (!enabled) [self resignFirstResponder];
        // Ready/re-enable is not user intent to focus. Only css focus may acquire it.
    }
}
- (void)didMoveToWindow {
    [super didMoveToWindow];
    [self.outsideObserver.view removeGestureRecognizer:self.outsideObserver];
    if (self.window && !self.disposed) {
        if (!self.outsideObserver) {
            self.outsideObserver = [[UITapGestureRecognizer alloc] initWithTarget:self action:@selector(observedOutside:)];
            self.outsideObserver.delegate = self;
            self.outsideObserver.cancelsTouchesInView = NO;
            self.outsideObserver.delaysTouchesBegan = NO;
            self.outsideObserver.delaysTouchesEnded = NO;
        }
        [self.window addGestureRecognizer:self.outsideObserver];
    } else [self resignFirstResponder];
}
- (void)observedOutside:(UITapGestureRecognizer *)recognizer { /* observer never recognizes */ }
- (BOOL)gestureRecognizer:(UIGestureRecognizer *)recognizer shouldReceiveTouch:(UITouch *)touch {
    if (self.isFirstResponder && self.superview && ![touch.view isDescendantOfView:self.superview])
        [self resignFirstResponder];
    return NO; // Do not consume or delay the underlying button/tab/text click.
}
- (void)pressesBegan:(NSSet<UIPress *> *)presses withEvent:(UIPressesEvent *)event {
    NSMutableSet *unhandled = [NSMutableSet set];
    for (UIPress *press in presses) {
        UIKey *key = press.key;
        if (!key) { [unhandled addObject:press]; continue; }
        UIKeyModifierFlags flags = key.modifierFlags;
        BOOL ctrl = (flags & UIKeyModifierControl) != 0;
        BOOL alt = (flags & UIKeyModifierAlternate) != 0;
        BOOL shift = (flags & UIKeyModifierShift) != 0;
        BOOL meta = (flags & UIKeyModifierCommand) != 0;
        NSString *name = nil;
        switch (key.keyCode) {
            case UIKeyboardHIDUsageKeyboardReturnOrEnter: name = @"Enter"; break;
            case UIKeyboardHIDUsageKeyboardEscape: name = @"Escape"; break;
            case UIKeyboardHIDUsageKeyboardTab: name = @"Tab"; break;
            case UIKeyboardHIDUsageKeyboardDeleteOrBackspace: name = @"Backspace"; break;
            case UIKeyboardHIDUsageKeyboardDeleteForward: name = @"Delete"; break;
            case UIKeyboardHIDUsageKeyboardUpArrow: name = @"ArrowUp"; break;
            case UIKeyboardHIDUsageKeyboardDownArrow: name = @"ArrowDown"; break;
            case UIKeyboardHIDUsageKeyboardLeftArrow: name = @"ArrowLeft"; break;
            case UIKeyboardHIDUsageKeyboardRightArrow: name = @"ArrowRight"; break;
            case UIKeyboardHIDUsageKeyboardHome: name = @"Home"; break;
            case UIKeyboardHIDUsageKeyboardEnd: name = @"End"; break;
            case UIKeyboardHIDUsageKeyboardPageUp: name = @"PageUp"; break;
            case UIKeyboardHIDUsageKeyboardPageDown: name = @"PageDown"; break;
            case UIKeyboardHIDUsageKeyboardInsert: name = @"Insert"; break;
            default:
                if (key.keyCode >= UIKeyboardHIDUsageKeyboardF1 && key.keyCode <= UIKeyboardHIDUsageKeyboardF12)
                    name = [NSString stringWithFormat:@"F%ld", (long)(key.keyCode - UIKeyboardHIDUsageKeyboardF1 + 1)];
                else if ((ctrl || alt) && !meta) name = key.charactersIgnoringModifiers;
                break;
        }
        if (meta || (ctrl && shift && [@[@"c", @"v"] containsObject:name.lowercaseString]) || !name) {
            [unhandled addObject:press]; continue;
        }
        [self emit:@{@"kind": @"key", @"key": name, @"ctrl": @(ctrl), @"alt": @(alt), @"shift": @(shift), @"meta": @(meta)}];
    }
    if (unhandled.count) [super pressesBegan:unhandled withEvent:event];
}
- (void)hrv_callWithMethod:(NSString *)method params:(NSString *)params callback:(KuiklyRenderCallback)callback {
    if ([method isEqualToString:@"focus"] && self.inputEnabled && !self.disposed) [self becomeFirstResponder];
    else if ([method isEqualToString:@"dispose"]) [self disposeInput];
}
- (void)disposeInput {
    self.disposed = YES; self.inputEnabled = NO;
    [self.outsideObserver.view removeGestureRecognizer:self.outsideObserver]; self.outsideObserver = nil;
    [self resignFirstResponder]; self.inputCallback = nil;
}
- (void)hrv_removeFromSuperview { [self disposeInput]; [self removeFromSuperview]; }
@end
