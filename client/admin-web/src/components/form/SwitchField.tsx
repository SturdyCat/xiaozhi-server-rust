/**
 * 布尔开关字段控件（`aiui.enabled` 等）。
 *
 * 草稿里值缺失或非布尔时按 `false` 显示；切换时用 `setField` 写布尔值，
 * 由 `diffPatch` 生成最小补丁。
 */

import type { ReactElement } from "react";
import { FieldRow } from "@/components/config/FieldRow";
import { Switch } from "@/components/ui/switch";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import type { FieldBaseProps } from "@/components/form/types";

export function SwitchField({
  path,
  label,
  hint,
  advanced,
}: FieldBaseProps): ReactElement {
  const { getField, setField } = useConfigDraft();
  const checked = getField(path) === true;

  return (
    <FieldRow label={label} hint={hint} advanced={advanced}>
      <Switch
        aria-label={label}
        checked={checked}
        onCheckedChange={(next) => setField(path, next)}
      />
    </FieldRow>
  );
}
