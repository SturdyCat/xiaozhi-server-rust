/**
 * 配置字段元数据 hook。
 *
 * 数据来自 `GET /api/config/schema`；这里只做两件派生：
 * - `isAdvancedField(path)`：字段是否属于「高级字段」（默认折叠）；
 * - `implementationsOf(section)`：某个能力下的实现列表。
 */

import { useCallback, useMemo } from "react";
import {
  getConfigSchema,
  type ConfigImplementationInfo,
  type ConfigSchemaInfo,
} from "@/lib/configApi";
import { useApiQuery } from "@/lib/useApiQuery";

/** 空实现列表的稳定引用，避免每次调用都新建数组。 */
const EMPTY_IMPLEMENTATIONS: ConfigImplementationInfo[] = [];

export interface ConfigSchemaState {
  schema: ConfigSchemaInfo | null;
  loading: boolean;
  error: string | null;
  isAdvancedField: (path: string) => boolean;
  implementationsOf: (section: string) => ConfigImplementationInfo[];
}

/** 拉取并派生配置字段元数据。 */
export function useConfigSchema(): ConfigSchemaState {
  const { data, loading, error } = useApiQuery(getConfigSchema);

  const advancedFields = useMemo(
    () => new Set(data?.advanced_fields ?? []),
    [data],
  );

  const isAdvancedField = useCallback(
    (path: string): boolean => advancedFields.has(path),
    [advancedFields],
  );

  const implementationsOf = useCallback(
    (section: string): ConfigImplementationInfo[] =>
      data?.capabilities.find((capability) => capability.id === section)
        ?.implementations ?? EMPTY_IMPLEMENTATIONS,
    [data],
  );

  return { schema: data, loading, error, isAdvancedField, implementationsOf };
}
