/**
 * 字段级重要提示条。
 *
 * 与 `FieldRow` 的 `hint`（`text-xs text-muted-foreground` 浅灰小字）区分：
 * 涉及鉴权、静态目录、云端链路、未知枚举值这类「误操作会出问题」的提示必须用它，
 * 不能降级成更浅的次级灰字。
 */

import type { ReactElement, ReactNode } from "react";
import { cn } from "@/lib/utils";

export type FormNoticeTone = "warning" | "danger" | "info";

export interface FormNoticeProps {
  tone?: FormNoticeTone;
  children: ReactNode;
}

const TONE_CLASS: Record<FormNoticeTone, string> = {
  warning: "border-amber-600/40 bg-amber-50 text-amber-900",
  danger: "border-destructive/40 bg-destructive/5 text-destructive",
  info: "border-sky-600/40 bg-sky-50 text-sky-900",
};

export function FormNotice({ tone = "warning", children }: FormNoticeProps): ReactElement {
  return (
    <div
      role={tone === "danger" ? "alert" : undefined}
      className={cn(
        "rounded-md border px-3 py-2 text-sm font-medium leading-relaxed",
        TONE_CLASS[tone],
      )}
    >
      {children}
    </div>
  );
}
