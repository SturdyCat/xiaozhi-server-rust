//! XiaoZhiModule 的「管理台实时通道」类别：独立于测试台 connect 的第二类 WebSocket。
//!
//! 为什么下沉到原生：Kuikly 的 Kotlin 侧运行在**独立 context 线程**，Kotlin 自建
//! NSURLSession 的回调无论投递到哪个 GCD 队列都不是 context 线程——断线重连一路
//! 调用共享层 `ctx.setTimeout` 就触发渲染器 `assertContextQueue` 断言（实测管理台
//! 整体卡死，2026-10「点击切换服务器卡死」事故根因）。而模块回调 block 由框架包裹
//! （KuiklyRenderCore `performCallback:` → `performOnContextQueueWithBlock`），原生在
//! 任意线程 invoke 都会被切回 context 线程再进 Kotlin。故 WS 生命周期事件全部经
//! `invoke:` 回调上行，Kotlin 侧（AdminRealtime actual）只保留状态机。
//!
//! 连接按 key 归属（Kotlin 侧生成自增 key），同 key 重复 connect 先关旧连接兜底；
//! 断旧连新/重连退避语义由 Kotlin 共享层管理，本类别只做无脑收发。

#import "XiaoZhiModule.h"

NS_ASSUME_NONNULL_BEGIN

@interface XiaoZhiModule (AdminWs)

/// adminWsConnect(key, url)：建立管理 WS。keepCallbackAlive 回调推生命周期事件：
/// {event:"open"} / {event:"text", data:文本帧} / {event:"closed"}。
- (void)adminWsConnect:(NSDictionary *)args;

/// adminWsSend(key, text)：发一帧文本（未连接静默丢弃）。
- (void)adminWsSend:(NSDictionary *)args;

/// adminWsClose(key)：关闭并移除连接（closed 事件随后仍回调一次）。
- (void)adminWsClose:(NSDictionary *)args;

@end

NS_ASSUME_NONNULL_END
