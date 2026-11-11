/**
 * 字段控件公共类型与小工具（W4 起各批配置段共用）。
 *
 * 约定：所有控件都通过 `useConfigDraft()` 按点分路径读写，绝不自己保存配置；
 * 标签 / 提示 / 高级门控统一交给已有的 `FieldRow` 渲染，控件只负责值与控制。
 */

import type { JsonValue } from "@/lib/configApi";

/** 所有字段控件的最小公共属性。 */
export interface FieldBaseProps {
  /** 配置树中的点分路径，例如 `server.port`。 */
  path: string;
  /** 中文标签。 */
  label: string;
  /** 常规说明（次要信息）；重要警示不要用 hint，改用 `FormNotice`。 */
  hint?: string;
  /** 是否为高级字段：简洁模式下整行不渲染。 */
  advanced?: boolean;
}

/** `SelectField` 的选项；`value` 统一用字符串表达，需要数字时由 `coerce` 转换。 */
export interface SelectOption {
  value: string;
  label: string;
}

/**
 * 把草稿值转成输入框可显示的字符串。
 *
 * 配置树里数字 / 布尔也可能落在文本字段上；对象与数组不当文本显示，回落 `fallback`。
 */
export function toText(value: JsonValue | undefined, fallback = ""): string {
  if (value === undefined || value === null) {
    return fallback;
  }
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return fallback;
}
