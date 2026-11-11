/**
 * 枚举字段控件。
 *
 * 当前草稿值不在 options 里时**只显示琥珀告警、不自动纠正、也不 setField**
 * （对齐 Kuikly `ConfigPlaybackCard.kt:35-38` 的「未知模式不静默改写」理由）。
 * `coerce` 用于「配置里存数字、选项用字符串表达」的场景（采样率 / 协议版本），
 * 避免把 `"24000"` 当字符串提交给只认 u32 的服务端。
 */

import type { ReactElement } from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { FieldRow } from "@/components/config/FieldRow";
import { FormNotice } from "@/components/form/FormNotice";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import type { JsonValue } from "@/lib/configApi";
import { toText, type FieldBaseProps, type SelectOption } from "@/components/form/types";

export interface SelectFieldProps extends FieldBaseProps {
  options: SelectOption[];
  /** 把选中的字符串转换成写入草稿的值；缺省按字符串写入。 */
  coerce?: (raw: string) => JsonValue;
}

export function SelectField({
  path,
  label,
  hint,
  advanced,
  options,
  coerce,
}: SelectFieldProps): ReactElement {
  const { getField, setField } = useConfigDraft();
  const current = toText(getField(path));
  const known = options.some((option) => option.value === current);
  // 未知值不喂给 Radix Select（否则显示空白且可能被误当成已选中）；用占位文案原样暴露。
  // 始终传字符串（含空串）保持受控，避免 uncontrolled → controlled 的切换告警。
  const selectValue = known ? current : "";

  return (
    <FieldRow label={label} hint={hint} advanced={advanced}>
      <Select
        value={selectValue}
        onValueChange={(next) => setField(path, coerce ? coerce(next) : next)}
      >
        <SelectTrigger className="w-[240px]">
          <SelectValue placeholder={current === "" ? "请选择" : `未识别取值：${current}`} />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem key={option.value} value={option.value}>
              {option.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {!known && current !== "" ? (
        <div className="mt-2">
          <FormNotice tone="warning">
            当前取值「{current}」不在可选列表中，界面不会自动改写；如不主动选择，保存时会原样提交。
          </FormNotice>
        </div>
      ) : null}
    </FieldRow>
  );
}
