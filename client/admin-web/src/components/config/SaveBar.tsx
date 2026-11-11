/**
 * 粘性底部保存条。
 *
 * `hot` 的三档文案由服务端 `impact.hot` 决定，客户端不自创第四种：
 * - live → 立即生效
 * - next_session → 对新会话生效
 * - restart_only → 需重启 server 生效
 */

import type { ReactElement } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export type SaveBarHot = "live" | "next_session" | "restart_only";

/** 三档 hot 的中文文案（唯一来源：总览段的「生效档位」卡也复用它，避免两处漂移）。 */
export const HOT_LABEL: Record<SaveBarHot, string> = {
  live: "立即生效",
  next_session: "对新会话生效",
  restart_only: "需重启 server 生效",
};

export interface SaveBarProps {
  dirtyCount: number;
  saving: boolean;
  hot?: SaveBarHot;
  onSave: () => void;
  onDiscard: () => void;
  message?: string;
  /**
   * 配置是否已成功加载（`revision !== null`）。为 false 时禁用保存并把文案改成
   * 「配置未加载，无法保存」——否则会显示「没有未保存的修改」，把「加载失败」说成「无改动」。
   */
  ready?: boolean;
}

export function SaveBar({
  dirtyCount,
  saving,
  hot,
  onSave,
  onDiscard,
  message,
  ready = true,
}: SaveBarProps): ReactElement {
  const clean = dirtyCount === 0;
  const disabled = saving || clean || !ready;

  return (
    <div
      className={cn(
        "sticky bottom-0 z-10 border-t bg-background/95 backdrop-blur",
        (clean || !ready) && "opacity-60",
      )}
    >
      <div className="mx-auto flex w-full max-w-5xl flex-wrap items-center justify-between gap-3 px-4 py-3">
        <div className="min-w-0 space-y-1">
          <p className="text-sm font-medium">
            {!ready
              ? "配置未加载，无法保存"
              : clean
                ? "没有未保存的修改"
                : `有 ${dirtyCount} 处未保存的修改`}
          </p>
          {message ? (
            <p className="truncate text-xs text-muted-foreground">{message}</p>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {hot && ready ? <Badge variant="outline">{HOT_LABEL[hot]}</Badge> : null}
          <Button
            variant="ghost"
            size="sm"
            onClick={onDiscard}
            disabled={disabled}
          >
            放弃修改
          </Button>
          <Button size="sm" onClick={onSave} disabled={disabled}>
            {saving ? "保存中…" : "保存配置"}
          </Button>
        </div>
      </div>
    </div>
  );
}
