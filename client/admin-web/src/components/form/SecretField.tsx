/**
 * 密钥字段控件（明文绝不进 draft）。
 *
 * 服务端 `SECRET_PATHS`（server/src/gateway/ws/config.rs:74-86）在 GET 时把密钥
 * **摘除**、只留 `has_<leaf>` presence（:128-145）；PUT 前置处理会把「空串密钥」
 * 从补丁里删掉 = 保留原值（:217-235）。因此客户端：
 * - 明文只存在 `secretValues`，由 `setSecret/getSecret` 管理；
 * - 输入空串 = 删掉该键 = 不提交；
 * - 保存时把非空明文展开成嵌套对象合并进 patch。
 */

import type { ReactElement } from "react";
import { FieldRow } from "@/components/config/FieldRow";
import { Input } from "@/components/ui/input";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import type { FieldBaseProps } from "@/components/form/types";

export interface SecretFieldProps extends FieldBaseProps {
  /** presence 标记路径，例如 `aiui.has_api_key`。 */
  hasPath: string;
  placeholder?: string;
}

export function SecretField({
  path,
  hasPath,
  label,
  hint,
  advanced,
  placeholder,
}: SecretFieldProps): ReactElement {
  const { getField, getSecret, setSecret } = useConfigDraft();
  const configured = getField(hasPath) === true;
  const value = getSecret(path) ?? "";
  const effectivePlaceholder = placeholder ?? (configured ? "已配置（留空保留）" : "未配置");

  return (
    <FieldRow label={label} hint={hint} advanced={advanced}>
      <Input
        type="password"
        autoComplete="new-password"
        className="max-w-md"
        value={value}
        placeholder={effectivePlaceholder}
        onChange={(event) => setSecret(path, event.target.value)}
      />
      <p className="mt-1 text-sm font-medium text-amber-800">
        {configured
          ? "服务端已保存该密钥；本次留空表示保留原值，填写则覆盖。"
          : "服务端未配置该密钥；留空表示不写入，填写后随本次保存提交。"}
      </p>
    </FieldRow>
  );
}
