/**
 * 轻量 Toast（自建，不引入 sonner）。
 *
 * 规格：
 * - 右下角堆叠，最多同时 4 条；
 * - 4 秒后自动消失；
 * - `aria-live="polite"`，读屏器以低优先级播报；
 * - error 用 `text-destructive`，success 用 `text-emerald-700`。
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactElement,
  type ReactNode,
} from "react";
import { cn } from "@/lib/utils";

/** Toast 语气。 */
export type ToastTone = "success" | "error" | "info";

interface ToastItem {
  id: number;
  message: string;
  tone: ToastTone;
}

export interface ToastApi {
  show: (message: string, tone?: ToastTone) => void;
}

/** 同时最多保留的条数。 */
const MAX_TOASTS = 4;
/** 自动消失时间（毫秒）。 */
const TOAST_TTL_MS = 4000;

const TONE_CLASS: Record<ToastTone, string> = {
  success: "text-emerald-700",
  error: "text-destructive",
  info: "text-foreground",
};

const ToastContext = createContext<ToastApi | null>(null);

/** 为子树提供 `useToast()`。 */
export function ToastProvider({ children }: { children: ReactNode }): ReactElement {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const nextId = useRef(1);
  const timers = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());

  const dismiss = useCallback((id: number): void => {
    setToasts((previous) => previous.filter((toast) => toast.id !== id));
    const timer = timers.current.get(id);
    if (timer !== undefined) {
      clearTimeout(timer);
      timers.current.delete(id);
    }
  }, []);

  const show = useCallback(
    (message: string, tone: ToastTone = "info"): void => {
      const trimmed = message.trim();
      if (trimmed === "") {
        return;
      }
      const id = nextId.current;
      nextId.current += 1;
      setToasts((previous) => {
        const next = [...previous, { id, message: trimmed, tone }];
        // 超出上限时丢最旧的一条；其定时器会在 TTL 后自然触发（dismiss 幂等）。
        return next.length > MAX_TOASTS ? next.slice(next.length - MAX_TOASTS) : next;
      });
      const timer = setTimeout(() => dismiss(id), TOAST_TTL_MS);
      timers.current.set(id, timer);
    },
    [dismiss],
  );

  useEffect(() => {
    const pending = timers.current;
    return () => {
      for (const timer of pending.values()) {
        clearTimeout(timer);
      }
      pending.clear();
    };
  }, []);

  const value = useMemo<ToastApi>(() => ({ show }), [show]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div
        aria-live="polite"
        aria-atomic="false"
        className="pointer-events-none fixed right-4 bottom-4 z-50 flex w-[min(24rem,calc(100vw-2rem))] flex-col gap-2"
      >
        {toasts.map((toast) => (
          <div
            key={toast.id}
            role="status"
            className={cn(
              "pointer-events-auto rounded-lg border bg-background px-4 py-3 text-sm shadow-lg",
              TONE_CLASS[toast.tone],
            )}
          >
            {toast.message}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

/** 读取 Toast API；必须在 `ToastProvider` 内使用。 */
export function useToast(): ToastApi {
  const value = useContext(ToastContext);
  if (value === null) {
    throw new Error("useToast 必须在 ToastProvider 内使用");
  }
  return value;
}
