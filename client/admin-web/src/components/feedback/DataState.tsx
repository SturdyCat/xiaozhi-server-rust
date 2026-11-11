/**
 * 数据状态占位：统一 loading / error / empty 三种呈现。
 * 优先级：loading > error > empty > children。
 */

import { CircleAlert, LoaderCircle } from "lucide-react";
import type { ReactElement, ReactNode } from "react";
import { Button } from "@/components/ui/button";

export interface DataStateProps {
  loading: boolean;
  error?: string | null;
  empty?: boolean;
  emptyText?: string;
  onRetry?: () => void;
  children: ReactNode;
}

export function DataState({
  loading,
  error,
  empty = false,
  emptyText = "暂无数据",
  onRetry,
  children,
}: DataStateProps): ReactElement {
  if (loading) {
    return (
      <div className="flex items-center gap-2 py-6 text-sm text-muted-foreground">
        <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
        <span>加载中…</span>
      </div>
    );
  }

  if (error) {
    return (
      <div
        role="alert"
        className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
      >
        <span className="flex min-w-0 items-center gap-2">
          <CircleAlert aria-hidden="true" className="size-4 shrink-0" />
          <span className="break-words">{error}</span>
        </span>
        {onRetry ? (
          <Button variant="outline" size="sm" onClick={onRetry}>
            重试
          </Button>
        ) : null}
      </div>
    );
  }

  if (empty) {
    return <div className="py-6 text-center text-sm text-muted-foreground">{emptyText}</div>;
  }

  return <>{children}</>;
}
