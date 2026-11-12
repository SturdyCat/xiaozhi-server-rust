import { useRequest } from "ahooks";
import { useState } from "react";

/**
 * 轻量查询 hook：封装一次异步拉取的生命周期（data / error / loading / updatedAt）。
 * 基于 ahooks `useRequest` 的薄封装，导出接口与手写实现完全一致。
 *
 * 并发保护：`useRequest` 默认只应用最新一次请求的结果（内部按请求序号丢弃过期响应），
 * 「快速切换目录/筛选条件」时，先发出的慢响应不会覆盖后发出的快响应。
 *
 * 语义约定：
 * - 挂载时自动请求一次；`deps` 变化时重新请求（`refreshDeps`）。
 * - `refetch` 手动重新发起请求；服务无参，等价于用最新 `queryFn` 闭包重跑一次。
 * - `error` 归一化为 `string`（`err.message` 或 `String(err)`），不是 Error 对象。
 * - `updatedAt` 是最近一次成功响应时的时间戳（毫秒），初始 `null`。
 * - `loading` 初始为 `true`（非 manual 模式），请求结束后为 `false`。
 *
 * @param queryFn 返回 Promise<T> 的查询函数（如 adminApi 的导出函数）。
 * @param deps    控制 refetch 身份的依赖项；为 [] 时 queryFn 仅在挂载时执行一次。
 */
export function useApiQuery<T>(
  queryFn: () => Promise<T>,
  deps: unknown[] = [],
): { data: T | null; error: string | null; loading: boolean; updatedAt: number | null; refetch: () => Promise<void> } {
  const [updatedAt, setUpdatedAt] = useState<number | null>(null);
  // 自行持有 error：对齐旧版「run 开始立即 setError(null)」的时序——ahooks 内部 error
  // 会残留到本次请求落定，切换 deps 加载期间会短暂显示上一轮旧错误。
  // （onBefore 对 mount / refreshDeps 重跑 / 手动 refetch 全部触发，见 Fetch.runAsync。）
  const [errorState, setErrorState] = useState<string | null>(null);
  const { data, loading, refreshAsync } = useRequest(queryFn, {
    refreshDeps: deps,
    onBefore: () => setErrorState(null),
    onSuccess: () => setUpdatedAt(Date.now()),
    onError: (err: unknown) => setErrorState(err instanceof Error ? err.message : String(err)),
  });
  return {
    data: (data ?? null) as T | null,
    error: errorState,
    loading,
    updatedAt,
    // 用 refreshAsync 拿到完整 Promise（refresh 的类型是 Promise | void 联合，无法链式）。
    // 归一化为 Promise<void> 且永不 reject——与原实现一致（原 run 的 .catch 吞掉错误、
    // 错误只写入 state；调用方 `await refetch()` 后读到的是已更新完的 data/error）。
    refetch: () =>
      refreshAsync()
        .then(() => undefined)
        .catch(() => undefined),
  };
}
