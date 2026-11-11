/**
 * 能力未启用提示：端点返回 404 / 501 时使用，避免把“未启用”伪装成“加载失败”。
 */

import { Info } from "lucide-react";
import type { ReactElement } from "react";

export interface UnavailableNoticeProps {
  title: string;
  description?: string;
}

export function UnavailableNotice({
  title,
  description,
}: UnavailableNoticeProps): ReactElement {
  return (
    <div className="flex items-start gap-3 rounded-lg border border-dashed border-border bg-muted/40 px-4 py-3">
      <Info aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0 space-y-1">
        <p className="text-sm font-medium">{title}</p>
        {description ? (
          <p className="text-xs text-muted-foreground">{description}</p>
        ) : null}
      </div>
    </div>
  );
}
