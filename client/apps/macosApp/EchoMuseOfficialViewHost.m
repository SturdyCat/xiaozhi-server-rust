#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>
#import <OpenKuiklyIOSRender/KuiklyRenderViewExportProtocol.h>

/**
 * Registered by Objective-C runtime class name (Kuikly's custom-view export convention).
 * Must exactly match the commonMain viewName and H5 renderViewExport name.
 */
@interface EchoMuseOfficialViewHost : UIView <KuiklyRenderViewExportProtocol, WKNavigationDelegate, WKUIDelegate>
@property (nonatomic, copy) NSString *documentURL;
@property (nonatomic, strong) WKWebView *webView;
@property (nonatomic, strong) UILabel *messageLabel;
@end

@implementation EchoMuseOfficialViewHost

- (void)hrv_setPropWithKey:(NSString *)propKey propValue:(id)propValue {
    KUIKLY_SET_CSS_COMMON_PROP;
    if ([propKey isEqualToString:@"url"]) {
        [self setDocumentURL:[propValue isKindOfClass:NSString.class] ? propValue : @""];
    }
}

- (void)hrv_callWithMethod:(NSString *)method params:(NSString *)params callback:(KuiklyRenderCallback)callback {
    if ([method isEqualToString:@"dispose"]) [self disposeDocument];
}

- (void)setDocumentURL:(NSString *)value {
    if ([_documentURL isEqualToString:value] && self.webView) return;
    [self disposeDocument];
    NSURL *url = [NSURL URLWithString:value];
    NSString *scheme = url.scheme.lowercaseString;
    if (!url.host.length || !([scheme isEqualToString:@"http"] || [scheme isEqualToString:@"https"])) {
        [self showMessage:@"官方管理页地址无效，需使用完整的 HTTP 或 HTTPS 地址。"];
        return;
    }
    _documentURL = [value copy];
    WKWebViewConfiguration *configuration = [[WKWebViewConfiguration alloc] init];
    // Keep document cookies/storage in this view's lifetime, not in the app's persistent store.
    configuration.websiteDataStore = WKWebsiteDataStore.nonPersistentDataStore;
    configuration.preferences.javaScriptCanOpenWindowsAutomatically = NO;
    self.webView = [[WKWebView alloc] initWithFrame:self.bounds configuration:configuration];
    self.webView.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
    self.webView.navigationDelegate = self;
    self.webView.UIDelegate = self;
    self.webView.allowsBackForwardNavigationGestures = NO;
    [self addSubview:self.webView];
    [self.webView loadRequest:[NSURLRequest requestWithURL:url]];
}

- (void)layoutSubviews {
    [super layoutSubviews];
    self.webView.frame = self.bounds;
    self.messageLabel.frame = CGRectInset(self.bounds, 16, 16);
}

- (BOOL)isDocumentOrigin:(NSURL *)url {
    NSURL *origin = [NSURL URLWithString:self.documentURL];
    NSNumber *originPort = origin.port ?: @([origin.scheme.lowercaseString isEqualToString:@"https"] ? 443 : 80);
    NSNumber *targetPort = url.port ?: @([url.scheme.lowercaseString isEqualToString:@"https"] ? 443 : 80);
    return origin && [url.scheme.lowercaseString isEqualToString:origin.scheme.lowercaseString]
        && [url.host.lowercaseString isEqualToString:origin.host.lowercaseString]
        && [targetPort isEqualToNumber:originPort];
}

- (void)webView:(WKWebView *)webView decidePolicyForNavigationAction:(WKNavigationAction *)action
    decisionHandler:(void (^)(WKNavigationActionPolicy))decisionHandler {
    // Do not send credentials to another origin, launch external schemes, or create new pages.
    BOOL allowed = [self isDocumentOrigin:action.request.URL];
    if (allowed && action.targetFrame == nil) {
        [webView loadRequest:action.request];
        decisionHandler(WKNavigationActionPolicyCancel);
        return;
    }
    decisionHandler(allowed ? WKNavigationActionPolicyAllow : WKNavigationActionPolicyCancel);
}

- (WKWebView *)webView:(WKWebView *)webView createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration
    forNavigationAction:(WKNavigationAction *)action windowFeatures:(WKWindowFeatures *)windowFeatures {
    // No new WKWebView/window/browser: navigation stays inside this host.
    if ([self isDocumentOrigin:action.request.URL]) [webView loadRequest:action.request];
    return nil;
}

- (void)webView:(WKWebView *)webView didFailProvisionalNavigation:(WKNavigation *)navigation withError:(NSError *)error {
    if (error.code != NSURLErrorCancelled) [self showMessage:@"官方管理页加载失败，请检查服务地址与网络后重新进入。"];
}

- (void)webView:(WKWebView *)webView didFinishNavigation:(WKNavigation *)navigation {
    [self.messageLabel removeFromSuperview];
    self.messageLabel = nil;
}

- (void)showMessage:(NSString *)message {
    if (!self.messageLabel) {
        self.messageLabel = [[UILabel alloc] initWithFrame:CGRectInset(self.bounds, 16, 16)];
        self.messageLabel.numberOfLines = 0;
        self.messageLabel.textAlignment = NSTextAlignmentCenter;
        self.messageLabel.textColor = UIColor.secondaryLabelColor;
        self.messageLabel.backgroundColor = UIColor.systemBackgroundColor;
        self.messageLabel.autoresizingMask = UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight;
        [self addSubview:self.messageLabel];
    }
    self.messageLabel.text = message;
    [self bringSubviewToFront:self.messageLabel];
}

- (void)disposeDocument {
    WKWebView *webView = self.webView;
    self.webView = nil;
    _documentURL = nil;
    webView.navigationDelegate = nil;
    webView.UIDelegate = nil;
    [webView stopLoading];
    // Replacing and releasing the document terminates its scripts and WebSocket/console leases.
    [webView loadHTMLString:@"<!doctype html><html></html>" baseURL:nil];
    [webView removeFromSuperview];
    [self.messageLabel removeFromSuperview];
    self.messageLabel = nil;
}

- (void)hrv_removeFromSuperview {
    [self disposeDocument];
    [self removeFromSuperview];
}

- (void)dealloc {
    [self disposeDocument];
}

@end
