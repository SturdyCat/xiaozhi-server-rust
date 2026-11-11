/**
 * 文本字段控件。
 *
 * `server.expected_token` 这类「GET 明文返回、留空 = 关闭鉴权」的字段就用它
 * （`type="password"` 只是遮挡显示），**不能**走 `SecretField` 的「留空保留」语义。
 */

import type { ReactElement } from "react";
import { FieldRow } from "@/components/config/FieldRow";
import { Input } from "@/components/ui/input";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import { toText, type FieldBaseProps } from "@/components/form/types";

export interface TextFieldProps extends FieldBaseProps {
  placeholder?: string;
  type?: "text" | "password";
}

export function TextField({
  path,
  label,
  hint,
  advanced,
  placeholder,
  type = "text",
}: TextFieldProps): ReactElement {
  const { getField, setField } = useConfigDraft();
  const value = toText(getField(path));

  return (
    <FieldRow label={label} hint={hint} advanced={advanced}>
      <Input
        type={type}
        className="max-w-md"
        value={value}
        placeholder={placeholder}
        autoComplete={type === "password" ? "new-password" : undefined}
        onChange={(event) => setField(path, event.target.value)}
      />
    </FieldRow>
  );
}
