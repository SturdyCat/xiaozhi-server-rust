import { useCallback, useEffect, useRef, useState } from "react";

/**
 * 轻量查询 hook：封装一次异步拉取的生命周期（data / error / loading / updatedAt）。
 * 不引入额外依赖，仅用 React 内置 hooks；后续页面可复用为数据获取样板。
 *
 * 并发保护：每次 `run()` 递增序号，只有最新一次请求的结果会被写入 state。
 * 否则「快速切换目录/筛选条件」时，先发出的慢响应会覆盖后发出的快响应。
 *
 * @param queryFn 返回 Promise<T> 的查询函数（如 adminApi 的导出函数）。
 * @param deps    控制 refetch 身份的依赖项；为 [] 时 queryFn 仅在挂载时执行一次。
 */
export function useApiQuery<T>(queryFn: () => Promise<T>, deps: unknown[] = []) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [updatedAt, setUpdatedAt] = useState<number | null>(null);
  /** 请求序号：只有最新一次请求允许写回 state。 */
  const seqRef = useRef(0);

  const run = useCallback(() => {
    const seq = seqRef.current + 1;
    seqRef.current = seq;
    setLoading(true);
    setError(null);
    return queryFn()
      .then((result) => {
        if (seq !== seqRef.current) {
          return;
        }
        setData(result);
        setUpdatedAt(Date.now());
      })
      .catch((err: unknown) => {
        if (seq !== seqRef.current) {
          return;
        }
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (seq !== seqRef.current) {
          return;
        }
        setLoading(false);
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  useEffect(() => {
    void run();
  }, [run]);

  return { data, error, loading, updatedAt, refetch: run };
}
