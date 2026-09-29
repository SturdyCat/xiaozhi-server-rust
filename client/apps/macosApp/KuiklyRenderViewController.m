#import "KuiklyRenderViewController.h"
#import <OpenKuiklyIOSRender/KuiklyRenderViewControllerBaseDelegator.h>
#import <OpenKuiklyIOSRender/KuiklyRenderContextProtocol.h>

@interface KuiklyRenderViewController () <KuiklyRenderViewControllerBaseDelegatorDelegate>
@property (nonatomic, strong) KuiklyRenderViewControllerBaseDelegator *delegator;
@end

@implementation KuiklyRenderViewController {
    NSDictionary *_pageData;
}

- (instancetype)initWithPageName:(NSString *)pageName pageData:(NSDictionary *)pageData {
    if (self = [super init]) {
        _pageData = pageData ?: @{};
        _delegator = [[KuiklyRenderViewControllerBaseDelegator alloc] initWithPageName:pageName
                                                                               pageData:_pageData];
        _delegator.delegate = self;
    }
    return self;
}

- (void)viewDidLoad {
    [super viewDidLoad];
    self.view.backgroundColor = [UIColor whiteColor];
    [_delegator viewDidLoadWithView:self.view];
    [self.navigationController setNavigationBarHidden:YES animated:NO];
}

- (void)viewDidLayoutSubviews { [super viewDidLayoutSubviews]; [_delegator viewDidLayoutSubviews]; }
- (void)viewWillAppear:(BOOL)animated { [super viewWillAppear:animated]; [_delegator viewWillAppear]; }
- (void)viewDidAppear:(BOOL)animated { [super viewDidAppear:animated]; [_delegator viewDidAppear]; }
- (void)viewWillDisappear:(BOOL)animated { [super viewWillDisappear:animated]; [_delegator viewWillDisappear]; }
- (void)viewDidDisappear:(BOOL)animated { [super viewDidDisappear:animated]; [_delegator viewDidDisappear]; }

#pragma mark - KuiklyRenderViewControllerBaseDelegatorDelegate

- (UIView *)createLoadingView {
    UIView *v = [[UIView alloc] init];
    v.backgroundColor = [UIColor whiteColor];
    return v;
}

- (UIView *)createErrorView {
    UIView *v = [[UIView alloc] init];
    v.backgroundColor = [UIColor whiteColor];
    return v;
}

// 返回业务代码编译成的 framework 名字（与 shared 的 baseName 一致）
- (void)fetchContextCodeWithPageName:(NSString *)pageName resultCallback:(KuiklyContextCodeCallback)callback {
    if (callback) callback(@"shared", nil);
}

- (UIWindow *)viewControllerHostWindow { return self.view.window; }

- (void)dealloc { [[NSNotificationCenter defaultCenter] removeObserver:self]; }

@end
