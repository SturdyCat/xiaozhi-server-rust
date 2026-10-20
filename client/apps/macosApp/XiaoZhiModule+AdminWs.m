#import "XiaoZhiModule+AdminWs.h"
#import "XiaoZhiModule+Internal.h"

// 连接注册表（key → client）：Kotlin 侧每次 adminWsConnect/Close 在 Kuikly 线程读写；
// 事件回调只在 URLSession 自建队列上触碰自己 client 的状态，不查表，无跨线程竞争。
@class XZAdminWsClient;
static NSMutableDictionary<NSString *, XZAdminWsClient *> *gAdminSockets = nil;

// 单条管理 WS 连接：session+delegate 持 open/closed 信号，receive 循环收文本帧。
// 独立小类而不是挂在模块属性上：管理通道与测试台 webSocket 互不牵连，数量与生命周期都归本类别管。
@interface XZAdminWsClient : NSObject
- (instancetype)initWithKey:(NSString *)key
                        url:(NSString *)url
                    onEvent:(void (^)(NSString *event, NSString *_Nullable data))onEvent;
- (void)open;
- (void)sendText:(NSString *)text;
- (void)close;
- (void)reportClosed;
@property (nonatomic, copy, readonly) NSString *key;
@property (nonatomic, copy, nullable) void (^onEvent)(NSString *event, NSString *_Nullable data);
@property (nonatomic, strong, nullable) NSURLSession *session;
@property (nonatomic, strong, nullable) NSURLSessionWebSocketTask *webSocket;
@property (nonatomic, assign) BOOL closedReported;
@end

// 生命周期代理：didOpen → open 事件；didClose → closed 事件（reportClosed 去重）。
// session 构造传入强引用，代理生命周期随 session，无提前释放问题。
@interface XZAdminWsDelegate : NSObject <NSURLSessionWebSocketDelegate>
- (instancetype)initWithClient:(__weak XZAdminWsClient *)client;
@property (nonatomic, weak, nullable) XZAdminWsClient *client;
@end

@implementation XZAdminWsClient

- (instancetype)initWithKey:(NSString *)key
                        url:(NSString *)url
                    onEvent:(void (^)(NSString *event, NSString *_Nullable data))onEvent {
    if ((self = [super init])) {
        _key = [key copy];
        _onEvent = [onEvent copy];
        __weak typeof(self) weak = self;
        _session = [NSURLSession
            sessionWithConfiguration:[NSURLSessionConfiguration defaultSessionConfiguration]
                            delegate:[[XZAdminWsDelegate alloc] initWithClient:weak]
                       delegateQueue:nil]; // session 自建串行队列；事件经 onEvent→invoke 由框架切回 Kuikly 线程
        _webSocket = [_session webSocketTaskWithURL:[NSURL URLWithString:url]];
    }
    return self;
}

- (void)open {
    [self.webSocket resume];
    [self receiveLoop];
}

- (void)sendText:(NSString *)text {
    if (!self.webSocket) return;
    [self.webSocket sendMessage:[[NSURLSessionWebSocketMessage alloc] initWithString:text]
              completionHandler:^(NSError *_Nullable e) {
        if (e) NSLog(@"[XiaoZhi][admin] send error: %@", e);
    }];
}

- (void)close {
    // cancel 会终结挂起的 receive（error 回包）→ reportClosed 去重后只回一次 closed。
    [self.webSocket cancelWithCloseCode:NSURLSessionWebSocketCloseCodeNormalClosure reason:nil];
    self.webSocket = nil;
}

- (void)receiveLoop {
    __weak typeof(self) weak = self;
    [self.webSocket receiveMessageWithCompletionHandler:^(NSURLSessionWebSocketMessage *_Nullable msg,
                                                          NSError *_Nullable error) {
        typeof(self) strong = weak;
        if (!strong || strong.webSocket == nil) return; // 已 close：不再续挂
        if (error) {
            [strong reportClosed];
            return;
        }
        NSString *text = msg.string;
        if (text.length > 0 && strong.onEvent) strong.onEvent(@"text", text);
        [strong receiveLoop];
    }];
}

// closed 至多回一次（delegate didClose / receive error / 主动 close 三路汇聚于此）。
- (void)reportClosed {
    if (self.closedReported) return;
    self.closedReported = YES;
    if (self.onEvent) self.onEvent(@"closed", nil);
}

@end

@implementation XZAdminWsDelegate

- (instancetype)initWithClient:(__weak XZAdminWsClient *)client {
    if ((self = [super init])) _client = client;
    return self;
}

- (void)URLSession:(NSURLSession *)session
    webSocketTask:(NSURLSessionWebSocketTask *)webSocketTask
didOpenWithProtocol:(NSString *)protocol {
    if (self.client.onEvent) self.client.onEvent(@"open", nil);
}

- (void)URLSession:(NSURLSession *)session
    webSocketTask:(NSURLSessionWebSocketTask *)webSocketTask
    didCloseWithCode:(NSURLSessionWebSocketCloseCode)code
    reason:(NSData *)reason {
    [self.client reportClosed];
}

@end

@implementation XiaoZhiModule (AdminWs)

- (void)adminWsConnect:(NSDictionary *)args {
    NSDictionary *params = [self parseParams:args[KR_PARAM_KEY]];
    id callback = args[KR_CALLBACK_KEY];
    NSString *key = params[@"key"] ?: @"";
    NSString *url = params[@"url"] ?: @"";
    if (key.length == 0 || url.length == 0) {
        [self invoke:callback result:@{@"event": @"closed"} success:NO error:@"invalid params"];
        return;
    }
    if (!gAdminSockets) gAdminSockets = [NSMutableDictionary dictionary];
    // 同 key 旧连接先关（共享层正常不会重复建连，这里兜底防泄漏）。
    XZAdminWsClient *old = gAdminSockets[key];
    if (old) {
        old.onEvent = nil; // 旧连接闭麦：closed 不再上送（Kotlin 侧已换代）
        [old close];
    }
    __weak typeof(self) weak = self;
    XZAdminWsClient *client =
        [[XZAdminWsClient alloc] initWithKey:key
                                          url:url
                                      onEvent:^(NSString *event, NSString *data) {
        NSMutableDictionary *res = [@{@"event": event} mutableCopy];
        if (data.length > 0) res[@"data"] = data;
        // invoke 的回调 block 由框架包裹：任意线程调用都会被切回 Kuikly context 线程。
        [weak invoke:callback result:res success:YES error:nil];
    }];
    gAdminSockets[key] = client;
    [client open];
}

- (void)adminWsSend:(NSDictionary *)args {
    NSDictionary *params = [self parseParams:args[KR_PARAM_KEY]];
    NSString *key = params[@"key"] ?: @"";
    NSString *text = params[@"text"] ?: @"";
    [gAdminSockets[key] sendText:text];
}

- (void)adminWsClose:(NSDictionary *)args {
    NSDictionary *params = [self parseParams:args[KR_PARAM_KEY]];
    NSString *key = params[@"key"] ?: @"";
    XZAdminWsClient *client = gAdminSockets[key];
    if (!client) return;
    [gAdminSockets removeObjectForKey:key];
    [client close];
}

@end
