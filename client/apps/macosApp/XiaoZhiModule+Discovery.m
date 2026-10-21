#import "XiaoZhiModule+Discovery.h"
#import "XiaoZhiModule+Internal.h"
#import <Network/Network.h>
#import <dns_sd.h>
#import <arpa/inet.h>

// 服务端广播的服务类型（mdns-sd 侧为 `_xiaozhi-admin._tcp.local.`；浏览时传类型 + 默认域即可）。
static NSString * const kAdminMdnsServiceType = @"_xiaozhi-admin._tcp";
// DNS-SD 的域名写法：带尾点，表示绝对域名。
static NSString * const kAdminMdnsDomain = @"local.";

// 单个实例的解析超时。DNS-SD 解析通常几十毫秒完成；超时说明该实例刚下线或本机拒绝
// 访问本地网络，收尾后列表不会永远挂着半条记录。
static const NSTimeInterval kResolveTimeoutSeconds = 3.0;

// 文件级强引用保住运行中的发现会话（同一时刻至多一个；start 覆盖旧会话 = 隐式 stop）。
@class XZServerDiscovery;
static XZServerDiscovery *gServerDiscovery = nil;

#pragma mark - 解析任务

/**
 * 一个在途解析任务：`DNSServiceResolve`（实例名 → 主机名 + 端口）
 * → `DNSServiceGetAddrInfo`（主机名 → IPv4）。
 *
 * 为什么不用 NWConnection 解析：browse 结果的端点是 bonjour_service（只有实例名），
 * 拿 ip:port 的旧做法是连一次服务端口再看 effective remote endpoint，但实测该连接
 * 会长期停在 `preparing`（既不 ready 也不 failed），发现列表因此永远为空。DNS-SD
 * 是纯查询、不建 TCP 连接，既快又不依赖连接权限。
 */
@interface XZResolveTask : NSObject
@property (nonatomic) DNSServiceRef resolveRef;
@property (nonatomic) DNSServiceRef addrRef;
@property (nonatomic, strong, nullable) dispatch_source_t resolveSource;
@property (nonatomic, strong, nullable) dispatch_source_t addrSource;
@property (nonatomic, copy) NSString *serviceName;
@property (nonatomic) uint16_t port;
@property (nonatomic, weak, nullable) XZServerDiscovery *owner;
@end

@implementation XZResolveTask
@end

#pragma mark - XZServerDiscovery

@interface XZServerDiscovery : NSObject
- (instancetype)initWithOnChange:(void (^)(NSArray<NSDictionary *> *servers))onChange;
- (void)start;
- (void)stop;
// DNS-SD 回调是 C 函数，经这几个方法回到 Objective-C（全部在 _queue 上执行）。
- (void)handleResolveTask:(XZResolveTask *)task host:(nullable NSString *)host port:(uint16_t)port;
- (void)handleResolveTask:(XZResolveTask *)task address:(nullable NSString *)ip;
- (void)finishTask:(XZResolveTask *)task reason:(nullable NSString *)reason;
- (dispatch_source_t)sourceForRef:(DNSServiceRef)ref;
@end

@implementation XZServerDiscovery {
    nw_browser_t _browser;
    dispatch_queue_t _queue;           // 全部可变状态只在它上面读写
    void (^_onChange)(NSArray<NSDictionary *> *servers);
    // 已解析服务器：key = "ip:port"，value = @{@"name":…, @"base":…}
    NSMutableDictionary<NSString *, NSDictionary *> *_servers;
    // 在途解析：key = 服务实例名（stop/替换时统一收尾）
    NSMutableDictionary<NSString *, XZResolveTask *> *_pending;
}

- (instancetype)initWithOnChange:(void (^)(NSArray<NSDictionary *> *servers))onChange {
    if ((self = [super init])) {
        _onChange = [onChange copy];
        _queue = dispatch_queue_create("com.xiaozhi.admin.serverdiscovery", DISPATCH_QUEUE_SERIAL);
        _servers = [NSMutableDictionary dictionary];
        _pending = [NSMutableDictionary dictionary];
    }
    return self;
}

- (void)start {
    nw_browse_descriptor_t descriptor = nw_browse_descriptor_create_bonjour_service(
        kAdminMdnsServiceType.UTF8String, NULL); // NULL 域 = 默认浏览域（local）
    _browser = nw_browser_create(descriptor, nw_parameters_create());
    __weak typeof(self) weak = self;
    nw_browser_set_queue(_browser, _queue);
    nw_browser_set_browse_results_changed_handler(
        _browser,
        ^(nw_browse_result_t oldResult, nw_browse_result_t newResult, bool batchComplete) {
            [weak handleResultChangeOld:oldResult new:newResult];
        });
    nw_browser_set_state_changed_handler(
        _browser,
        ^(nw_browser_state_t state, nw_error_t _Nullable error) {
            if (state == nw_browser_state_failed) {
                // 最常见的失败原因就是本地网络权限被拒（macOS 15+ 强制）：此时系统
                // 不会弹窗，浏览与后续解析一起被静默拦截，只能在日志里留下痕迹。
                NSLog(@"[XiaoZhi][discovery] 浏览失败: %@（若已允许服务端启动，请检查 "
                      @"系统设置 → 隐私与安全性 → 本地网络 是否允许本 App）",
                      error ?: @"未知原因");
            }
        });
    nw_browser_start(_browser);
    NSLog(@"[XiaoZhi][discovery] 开始浏览 %@", kAdminMdnsServiceType);
}

- (void)stop {
    if (_browser) {
        nw_browser_cancel(_browser);
        _browser = NULL;
    }
    // 在途解析全部收尾（deallocate 后 DNS-SD 不再回调）。
    for (XZResolveTask *task in _pending.allValues) [self finishTask:task reason:nil];
    [_pending removeAllObjects];
    BOOL hadServers = _servers.count > 0;
    [_servers removeAllObjects];
    if (hadServers && _onChange) _onChange(@[]); // 收尾清一次列表（面板通常已关闭，幂等无害）
    NSLog(@"[XiaoZhi][discovery] 停止浏览");
}

#pragma mark 结果差异

- (void)handleResultChangeOld:(nw_browse_result_t)oldResult new:(nw_browse_result_t)newResult {
    nw_browse_result_change_t changes = nw_browse_result_get_changes(oldResult, newResult);
    if (changes & nw_browse_result_change_result_removed) {
        // 移除：按旧结果的服务实例名清掉在途解析与已解析条目。
        NSString *name = [self serviceNameOf:oldResult];
        XZResolveTask *task = _pending[name];
        if (task) [self finishTask:task reason:nil];
        [self removeServersNamed:name];
    } else if (newResult) {
        // 新增/更新（TXT、接口变化等）：重新解析出最新 ip:port。
        [self resolveResult:newResult];
    }
    [self notifyChanged];
}

- (NSString *)serviceNameOf:(nw_browse_result_t)result {
    nw_endpoint_t endpoint = nw_browse_result_copy_endpoint(result);
    NSString *name = [NSString stringWithUTF8String:nw_endpoint_get_bonjour_service_name(endpoint)];
    return name ?: @"";
}

- (void)removeServersNamed:(NSString *)name {
    NSMutableArray<NSString *> *stale = [NSMutableArray array];
    for (NSString *key in _servers) {
        if ([_servers[key][@"name"] isEqualToString:name]) [stale addObject:key];
    }
    for (NSString *key in stale) [_servers removeObjectForKey:key];
}

#pragma mark 解析（服务实例 → ip:port，纯 DNS-SD 查询）

static void XZResolveCallback(DNSServiceRef sdRef, DNSServiceFlags flags, uint32_t ifIndex,
                              DNSServiceErrorType err, const char *fullname, const char *hosttarget,
                              uint16_t port, uint16_t txtLen, const unsigned char *txt,
                              void *context);
static void XZAddrCallback(DNSServiceRef sdRef, DNSServiceFlags flags, uint32_t ifIndex,
                           DNSServiceErrorType err, const char *hostname,
                           const struct sockaddr *address, uint32_t ttl, void *context);

- (void)resolveResult:(nw_browse_result_t)result {
    NSString *name = [self serviceNameOf:result];
    if (name.length == 0) return;
    XZResolveTask *old = _pending[name];
    if (old) [self finishTask:old reason:nil];

    XZResolveTask *task = [XZResolveTask new];
    task.serviceName = name;
    task.owner = self;
    DNSServiceRef ref = NULL;
    DNSServiceErrorType err = DNSServiceResolve(&ref, 0, kDNSServiceInterfaceIndexAny,
                                                name.UTF8String, kAdminMdnsServiceType.UTF8String,
                                                kAdminMdnsDomain.UTF8String,
                                                XZResolveCallback, (__bridge void *)task);
    if (err != kDNSServiceErr_NoError || !ref) {
        NSLog(@"[XiaoZhi][discovery] 解析 %@ 发起失败: %d", name, (int)err);
        return;
    }
    task.resolveRef = ref;
    task.resolveSource = [self sourceForRef:ref];
    _pending[name] = task;

    // 超时兜底：实例刚下线 / 本地网络被拦时不会有正常结果，避免永远挂在 _pending。
    __weak typeof(self) weak = self;
    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(kResolveTimeoutSeconds * NSEC_PER_SEC)),
        _queue, ^{
            XZServerDiscovery *strongSelf = weak;
            if (!strongSelf) return;
            XZResolveTask *still = strongSelf->_pending[name];
            if (still == task) [strongSelf finishTask:task reason:@"超时"];
        });
}

/** 把 DNS-SD 的 socket 挂到 _queue 上驱动（串行队列保证状态无竞争）。 */
- (dispatch_source_t)sourceForRef:(DNSServiceRef)ref {
    int fd = DNSServiceRefSockFD(ref);
    dispatch_source_t source = dispatch_source_create(DISPATCH_SOURCE_TYPE_READ, (uintptr_t)fd, 0, _queue);
    dispatch_source_set_event_handler(source, ^{
        DNSServiceProcessResult(ref);
    });
    dispatch_resume(source);
    return source;
}

- (void)handleResolveTask:(XZResolveTask *)task host:(NSString *)host port:(uint16_t)port {
    if (task.port != 0) return;      // 同一实例的重复回调：只在首次（拿到端口后）继续
    if (host.length == 0 || port == 0) {
        [self finishTask:task reason:@"SRV 记录缺少主机或端口"];
        return;
    }
    task.port = port;
    DNSServiceRef addrRef = NULL;
    DNSServiceErrorType err = DNSServiceGetAddrInfo(&addrRef, 0, kDNSServiceInterfaceIndexAny,
                                                    kDNSServiceProtocol_IPv4, host.UTF8String,
                                                    XZAddrCallback, (__bridge void *)task);
    if (err != kDNSServiceErr_NoError || !addrRef) {
        [self finishTask:task reason:@"A 记录查询发起失败"];
        return;
    }
    task.addrRef = addrRef;
    task.addrSource = [self sourceForRef:addrRef];
}

- (void)handleResolveTask:(XZResolveTask *)task address:(NSString *)ip {
    if (ip.length == 0 || task.port == 0) {
        [self finishTask:task reason:@"A 记录为空"];
        return;
    }
    NSString *key = [NSString stringWithFormat:@"%@:%u", ip, task.port];
    _servers[key] = @{
        @"name": task.serviceName,
        @"base": [NSString stringWithFormat:@"http://%@:%u", ip, task.port],
    };
    NSLog(@"[XiaoZhi][discovery] 已解析 %@ → http://%@:%u", task.serviceName, ip, task.port);
    [self finishTask:task reason:nil];
    // ⚠️ 必须在这里通知一次：browse 的 add 事件早于解析完成，那一刻 _servers 还是空的
    // （已经回调过一次空列表）。不补这次通知，列表就停在「未发现」、自动接管也永远
    // 不触发——browser 后续未必再发事件。
    [self notifyChanged];
}

- (void)finishTask:(XZResolveTask *)task reason:(NSString *)reason {
    if (task.resolveSource) {
        dispatch_source_cancel(task.resolveSource);
        task.resolveSource = nil;
    }
    if (task.addrSource) {
        dispatch_source_cancel(task.addrSource);
        task.addrSource = nil;
    }
    if (task.resolveRef) {
        DNSServiceRefDeallocate(task.resolveRef);
        task.resolveRef = NULL;
    }
    if (task.addrRef) {
        DNSServiceRefDeallocate(task.addrRef);
        task.addrRef = NULL;
    }
    if (task.serviceName.length > 0 && _pending[task.serviceName] == task) {
        [_pending removeObjectForKey:task.serviceName];
    }
    if (reason.length > 0) {
        NSLog(@"[XiaoZhi][discovery] 解析 %@ 未完成：%@", task.serviceName, reason);
    }
}

- (void)notifyChanged {
    if (!_onChange) return;
    // 按实例名稳定排序，UI 列表不因解析完成顺序抖动。
    NSArray<NSDictionary *> *sorted = [_servers.allValues
        sortedArrayUsingComparator:^NSComparisonResult(NSDictionary *a, NSDictionary *b) {
            return [a[@"name"] compare:b[@"name"]];
        }];
    _onChange(sorted);
}

@end

#pragma mark - DNS-SD 回调

static void XZResolveCallback(DNSServiceRef sdRef, DNSServiceFlags flags, uint32_t ifIndex,
                              DNSServiceErrorType err, const char *fullname, const char *hosttarget,
                              uint16_t port, uint16_t txtLen, const unsigned char *txt,
                              void *context) {
    XZResolveTask *task = (__bridge XZResolveTask *)context;
    XZServerDiscovery *owner = task.owner;
    if (!owner) return;
    if (err != kDNSServiceErr_NoError) {
        [owner finishTask:task reason:[NSString stringWithFormat:@"SRV 解析失败 %d", (int)err]];
        return;
    }
    NSString *host = hosttarget ? [NSString stringWithUTF8String:hosttarget] : nil;
    // SRV 的端口是网络字节序。
    [owner handleResolveTask:task host:host port:ntohs(port)];
}

static void XZAddrCallback(DNSServiceRef sdRef, DNSServiceFlags flags, uint32_t ifIndex,
                           DNSServiceErrorType err, const char *hostname,
                           const struct sockaddr *address, uint32_t ttl, void *context) {
    XZResolveTask *task = (__bridge XZResolveTask *)context;
    XZServerDiscovery *owner = task.owner;
    if (!owner) return;
    if (err != kDNSServiceErr_NoError) {
        [owner finishTask:task reason:[NSString stringWithFormat:@"A 记录解析失败 %d", (int)err]];
        return;
    }
    char buf[INET_ADDRSTRLEN] = {0};
    if (!address || address->sa_family != AF_INET) {
        [owner finishTask:task reason:@"A 记录不含 IPv4 地址"];
        return;
    }
    inet_ntop(AF_INET, &((const struct sockaddr_in *)address)->sin_addr, buf, sizeof(buf));
    [owner handleResolveTask:task address:[NSString stringWithUTF8String:buf]];
}

#pragma mark - Module 方法

@implementation XiaoZhiModule (Discovery)

- (void)startServerDiscovery:(NSDictionary *)args {
    id callback = args[KR_CALLBACK_KEY];
    // 重入（面板已开再点入口/自动接管并发）：先停旧会话再起新的，回调换绑最新 block。
    [gServerDiscovery stop];
    __weak typeof(self) weak = self;
    gServerDiscovery = [[XZServerDiscovery alloc] initWithOnChange:^(NSArray<NSDictionary *> *servers) {
        // 回调 block 由框架包裹：任意线程调用都会被切回 Kuikly context 线程。
        [weak invoke:callback result:@{@"servers": servers ?: @[]} success:YES error:nil];
    }];
    [gServerDiscovery start];
}

- (void)stopServerDiscovery:(NSDictionary *)args {
    [gServerDiscovery stop];
    gServerDiscovery = nil;
}

@end
