#import "XiaoZhiModule+Discovery.h"
#import "XiaoZhiModule+Internal.h"
#import <Network/Network.h>

// 服务端广播的服务类型（mdns-sd 侧为 `_xiaozhi-admin._tcp.local.`；浏览时传类型 + 默认域即可）。
static NSString * const kAdminMdnsServiceType = @"_xiaozhi-admin._tcp";

// 文件级强引用保住运行中的发现会话（同一时刻至多一个；start 覆盖旧会话 = 隐式 stop）。
@class XZServerDiscovery;
static XZServerDiscovery *gServerDiscovery = nil;

#pragma mark - XZServerDiscovery

@interface XZServerDiscovery : NSObject
- (instancetype)initWithOnChange:(void (^)(NSArray<NSDictionary *> *servers))onChange;
- (void)start;
- (void)stop;
@end@implementation XZServerDiscovery {
    nw_browser_t _browser;
    dispatch_queue_t _queue;           // 全部可变状态（servers/pending）只在它上面读写
    void (^_onChange)(NSArray<NSDictionary *> *servers);
    // 已解析服务器：key = "ip:port"，value = @{@"name":…, @"base":…}
    NSMutableDictionary<NSString *, NSDictionary *> *_servers;
    // 在途解析：key = 服务实例名，value = 用于解析的 NWConnection（stop/替换时取消）
    NSMutableDictionary<NSString *, nw_connection_t> *_pending;
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
                NSLog(@"[XiaoZhi][discovery] 浏览失败: %@", error ?: nil);
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
    for (nw_connection_t conn in _pending.allValues) nw_connection_cancel(conn);
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
        // 移除：newResult 为空，按旧结果的服务实例名清掉它的解析与已解析条目。
        NSString *name = [self serviceNameOf:oldResult];
        nw_connection_t pending = _pending[name];
        if (pending) {
            nw_connection_cancel(pending);
            [_pending removeObjectForKey:name];
        }
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

#pragma mark 解析（服务实例 → ip:port）

// NWConnection 解析舞步：browse 结果端点是 bonjour_service 类型（只有实例名），
// 真实 ip:port 要等连接 ready 后从 effective remote endpoint 拿。建连对象是服务端
// 的 HTTP 管理端口——连上即断，服务端无感。
- (void)resolveResult:(nw_browse_result_t)result {
    NSString *name = [self serviceNameOf:result];
    if (name.length == 0) return;
    nw_connection_t old = _pending[name];
    if (old) {
        nw_connection_cancel(old);
        [_pending removeObjectForKey:name];
    }
    nw_endpoint_t endpoint = nw_browse_result_copy_endpoint(result);
    nw_connection_t conn = nw_connection_create(endpoint, nw_parameters_create());
    if (!conn) return;
    // 解析回调锁死到 _queue：_servers/_pending 只在它上面读写（不设队列则回调落在
    // Network.framework 私有队列，与浏览器事件竞争可变状态）。
    nw_connection_set_queue(conn, _queue);
    __weak typeof(self) weak = self;
    nw_connection_set_state_changed_handler(conn, ^(nw_connection_state_t state, nw_error_t error) {
        [weak connection:conn stateChanged:state name:name];
    });
    _pending[name] = conn;
    nw_connection_start(conn);
}

- (void)connection:(nw_connection_t)conn stateChanged:(nw_connection_state_t)state name:(NSString *)name {
    switch (state) {
        case nw_connection_state_ready: {
            nw_path_t path = nw_connection_copy_current_path(conn);
            nw_endpoint_t remote = path ? nw_path_copy_effective_remote_endpoint(path) : NULL;
            if (remote) {
                NSString *ip = [NSString stringWithUTF8String:nw_endpoint_get_hostname(remote)];
                uint16_t port = nw_endpoint_get_port(remote);
                if (ip.length > 0 && port > 0) {
                    _servers[[NSString stringWithFormat:@"%@:%u", ip, port]] = @{
                        @"name": name,
                        @"base": [NSString stringWithFormat:@"http://%@:%u", ip, port],
                    };
                }
            }
            nw_connection_cancel(conn);
            break;
        }
        case nw_connection_state_failed:
        case nw_connection_state_cancelled:
            if (_pending[name] == conn) [_pending removeObjectForKey:name];
            break;
        default:
            break; // preparing/waiting：等最终态
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
