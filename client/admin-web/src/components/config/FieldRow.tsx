/**
 * 配置字段行：标签 + 提示 + 可选徽标 + 控件。
 *
 * `advanced` 字段在高级模式关闭时整体不渲染；`overrideAdvanced` 用于少数必须
 * 始终可见的高级字段（例如安全相关开关）。
 */

import type { ReactElement, ReactNode } from "react";
import { Label } from "@/components/ui/label";
import { useAdvancedMode } from "@/context/AdvancedModeContext";

export interface FieldRowProps {
  label: string;
  hint?: string;
  advanced?: boolean;
  overrideAdvanced?: boolean;
  badge?: ReactNode;
  children: ReactNode;
}

export function FieldRow({
  label,
  hint,
  advanced = false,
  overrideAdvanced = false,
  badge,
  children,
}: FieldRowProps): ReactElement | null {
  const { advancedMode } = useAdvancedMode();

  if (advanced && !advancedMode && !overrideAdvanced) {
    return null;
  }

  return (
    <div className="grid gap-1.5 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <Label className="text-sm font-medium">{label}</Label>
        {badge}
      </div>
      {hint ? <p className="text-xs text-muted-foreground">{hint}</p> : null}
      <div className="pt-1">{children}</div>
    </div>
  );
}
