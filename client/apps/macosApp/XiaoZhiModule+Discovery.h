//! XiaoZhiModule 的「局域网服务器发现」类别：Bonjour 浏览服务端常驻广播的
//! `_xiaozhi-admin._tcp`（服务端实现见 server/src/app/discovery.rs，与 EchoMuse 的
//! `_emcontroller._tcp` 同机制、不同类型），解析出 `http://<ip>:<port>` API 基址列表。
//!
//! 放原生侧的两个理由：
//! 1. **线程**：发现事件与「服务器设置」面板的 observable 更新都必须发生在 Kuikly
//!    context 线程——经模块回调 block 上行由框架 `performOnContextQueue` 兜底切换，
//!    Kotlin 侧零线程代码（与 AdminWs 同一套 threading 故事）。
//! 2. **能力**：Bonjour（NWBrowser + NWConnection 解析）是 Network.framework 独占 API，
//!    Kuikly core 无对应跨端模块。
//!
//! 事件语义：每次发现列表变化回调一次 {servers:[{name, base}]}（全量列表，Kotlin 侧
//! 整体替换 vfor 数据源）；stopServerDiscovery 后回调闭麦、资源全部释放。

#import "XiaoZhiModule.h"

NS_ASSUME_NONNULL_BEGIN

@interface XiaoZhiModule (Discovery)

/// startServerDiscovery()：开始浏览（幂等，重复调用重起）。keepCallbackAlive 回调推全量列表。
- (void)startServerDiscovery:(NSDictionary *)args;

/// stopServerDiscovery()：停止浏览并释放（回调不再触发）。
- (void)stopServerDiscovery:(NSDictionary *)args;

@end

NS_ASSUME_NONNULL_END
