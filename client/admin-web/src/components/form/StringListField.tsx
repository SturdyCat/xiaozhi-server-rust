/**
 * 字符串数组字段控件（`command.keywords` 这类 `Vec<String>`），用换行分隔编辑。
 *
 * - 非空行 `trim()` 后整体写成 `string[]` 回 draft；`diffPatch` 对数组整体替换，
 *   与服务端 `merge_patch`「非对象整体替换」语义一致（server/src/gateway/ws/config.rs:414-430）。
 * - 读取时对非数组值安全降级成空数组，绝不抛错。
 * - 本地文本 state 保留打字中间态（末尾空行、行内空格），只在外部 reload / discard /
 *   保存回读导致 draft 变化时同步显示。
 */

import { useEffect, useRef, useState, type ReactElement } from "react";
import { FieldRow } from "@/components/config/FieldRow";
import { Textarea } from "@/components/ui/textarea";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import type { JsonValue } from "@/lib/configApi";
import type { FieldBaseProps } from "@/components/form/types";

export interface StringListFieldProps extends FieldBaseProps {
  placeholder?: string;
  rows?: number;
}

/** 安全读取草稿里的字符串数组：非数组、非字符串元素都降级跳过，绝不崩。 */
export function stringListOf(value: JsonValue | undefined): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const lines: string[] = [];
  for (const item of value) {
    if (typeof item === "string") {
      lines.push(item);
    }
  }
  return lines;
}

/** 把文本框内容解析成数组：按换行拆分、逐行 trim、丢弃空行。 */
function parseLines(raw: string): string[] {
  return raw
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

export function StringListField({
  path,
  label,
  hint,
  advanced,
  placeholder,
  rows = 5,
}: StringListFieldProps): ReactElement {
  const { getField, setField } = useConfigDraft();
  const draftText = stringListOf(getField(path)).join("\n");
  const [text, setText] = useState<string>(draftText);
  const lastCommitted = useRef<string>(draftText);

  useEffect(() => {
    if (draftText !== lastCommitted.current) {
      lastCommitted.current = draftText;
      setText(draftText);
    }
  }, [draftText]);

  const handleChange = (raw: string): void => {
    setText(raw);
    const lines = parseLines(raw);
    lastCommitted.current = lines.join("\n");
    setField(path, lines);
  };

  const count = parseLines(text).length;

  return (
    <FieldRow label={label} hint={hint} advanced={advanced}>
      <Textarea
        className="max-w-2xl"
        rows={rows}
        value={text}
        placeholder={placeholder}
        onChange={(event) => handleChange(event.target.value)}
      />
      <p className="mt-1 text-sm font-medium">共 {count} 条</p>
    </FieldRow>
  );
}
