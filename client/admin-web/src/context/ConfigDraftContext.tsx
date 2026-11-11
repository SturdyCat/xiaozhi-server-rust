/**
 * 配置草稿状态层：baseline / draft / revision / 保存与冲突处理。
 *
 * 设计要点：
 * - baseline 是服务端最近一次确认的配置（含 revision），draft 是用户正在编辑的副本；
 * - `save()` 用 `diffPatch` 只提交变化叶子，并带上 revision 做乐观并发控制；
 * - 密钥明文只存在 `secretValues`，**绝不进 draft**：GET 把密钥摘成 `has_*` presence，
 *   明文若混入 draft 会被当成真实配置回写；保存时再展开成嵌套路径合并进 patch；
 * - **保存成功后必须回读 `getConfig()`**：GET 会重新做密钥打码（`has_*` 才准）、
 *   服务端还会 normalize 并丢弃「空串密钥」这类不生效的写法，拿本地 draft 冒充 baseline
 *   会让「清空密钥」显示成功而实际仍生效。回读失败走 `warning`（保存本身成功）；
 * - 409 冲突时**保留 draft**、置 `conflict`，不自动重载，避免用户输入被静默清空；
 * - 请求在途期间用户可能继续输入，回包后只在 draft 未被并发修改时才重置。
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactElement,
  type ReactNode,
} from "react";
import {
  ConfigConflictError,
  configErrorMessage,
  getConfig,
  putConfig,
  type ConfigRoot,
  type ConfigSaveResult,
  type JsonObject,
  type JsonValue,
} from "@/lib/configApi";
import { countDirty, diffPatch, dirtyPathsOf } from "@/lib/configDiff";

export interface ConfigDraftValue {
  /** 服务端最近确认的配置；首屏加载完成前为 null。 */
  readonly baseline: ConfigRoot | null;
  /** 用户编辑中的配置副本（不含包装字段 revision，也不含任何密钥明文）。 */
  readonly draft: JsonObject;
  /** 当前草稿所基于的内容指纹。 */
  readonly revision: string | null;
  readonly saving: boolean;
  readonly loading: boolean;
  /** 最近一次加载/保存的失败原因。 */
  readonly error: string | null;
  /** 非致命提示（例如「已保存但回读失败」）：保存动作本身成功，但本地状态可能过期。 */
  readonly warning: string | null;
  /** 是否处于 409 并发冲突状态（draft 仍保留）。 */
  readonly conflict: boolean;
  /** 最近一次成功保存的响应。 */
  readonly lastResult: ConfigSaveResult | null;
  /**
   * 待写入的密钥明文（点分路径 → 明文）。**绝不放进 draft**：GET 打码后 draft 里
   * 只有 `has_*` 标记，明文只能存在这里，保存时才展开进 patch。
   */
  readonly secretValues: Record<string, string>;
  /** 草稿相对 baseline 的变化叶子数（含「只改密钥」的改动）。 */
  readonly dirtyCount: number;
  /** 本次改动的叶子点分路径清单（含密钥路径），供简洁模式高级字段二次确认。 */
  readonly dirtyPaths: string[];

  setField: (dottedPath: string, value: JsonValue) => void;
  getField: (dottedPath: string) => JsonValue | undefined;
  /** 写入密钥明文；空串表示删除该键（= 不提交、保留服务端原值）。 */
  setSecret: (dottedPath: string, value: string) => void;
  /** 读取尚未提交的密钥明文；未填写返回 undefined。 */
  getSecret: (dottedPath: string) => string | undefined;
  /** 保存变化；无改动或未加载完成时返回 null。冲突时保留 draft 并返回 null。 */
  save: () => Promise<ConfigSaveResult | null>;
  /** 放弃本地改动，草稿回到当前 baseline（密钥明文一并清空）。 */
  discard: () => void;
  /** 重新拉取服务端配置，覆盖 baseline 与 draft。返回是否成功（失败时已置 `error`）。 */
  reload: () => Promise<boolean>;
  /** 冲突解决：重载服务端配置并放弃本地改动。 */
  resolveConflict: () => Promise<void>;
}

const ConfigDraftContext = createContext<ConfigDraftValue | null>(null);

/** 判断是否为普通 JSON 对象（数组不算）。 */
function isJsonObject(value: JsonValue | undefined): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 深拷贝 JSON 值，避免草稿与 baseline 共享嵌套对象。 */
function cloneJson(value: JsonValue): JsonValue {
  if (Array.isArray(value)) {
    return value.map(cloneJson);
  }
  if (isJsonObject(value)) {
    const copy: JsonObject = {};
    for (const [key, item] of Object.entries(value)) {
      copy[key] = cloneJson(item);
    }
    return copy;
  }
  return value;
}

/** 复制一个 JSON 对象（深拷贝）。 */
function cloneObject(value: JsonObject): JsonObject {
  const copy: JsonObject = {};
  for (const [key, item] of Object.entries(value)) {
    copy[key] = cloneJson(item);
  }
  return copy;
}

/** 从服务端响应里剥掉包装字段 revision，得到可编辑的草稿。 */
function toDraft(root: ConfigRoot): JsonObject {
  const draft: JsonObject = {};
  for (const [key, value] of Object.entries(root)) {
    if (key === "revision") {
      continue;
    }
    draft[key] = cloneJson(value);
  }
  return draft;
}

/** 按点分路径读取值；中途类型不匹配时返回 undefined。 */
function readPath(root: JsonValue | undefined, segments: string[]): JsonValue | undefined {
  let current: JsonValue | undefined = root;
  for (const segment of segments) {
    if (isJsonObject(current)) {
      current = current[segment];
    } else if (Array.isArray(current)) {
      current = current[Number(segment)];
    } else {
      return undefined;
    }
  }
  return current;
}

/** 按点分路径不可变写入；中间缺失的对象会自动补出来。 */
function writePath(root: JsonObject, segments: string[], value: JsonValue): JsonObject {
  const [head, ...rest] = segments;
  if (head === undefined) {
    return root;
  }
  if (rest.length === 0) {
    return { ...root, [head]: value };
  }
  const current = root[head];
  const child = isJsonObject(current) ? current : {};
  return { ...root, [head]: writePath(child, rest, value) };
}

/** 把 "a.b.c" 拆成路径段；忽略空段。 */
function splitPath(dottedPath: string): string[] {
  return dottedPath.split(".").filter((segment) => segment.length > 0);
}

/**
 * 把密钥明文字典展开成嵌套对象合并进 patch。
 *
 * 服务端 `merge_patch` 按嵌套路径递归解析（server/src/config.rs），不能塞
 * `"aiui.api_key"` 这种扁平 key；`writePath` 正好做不可变嵌套写入。
 * 空串一律跳过：服务端 `sanitize_patch` 会把空串密钥从补丁里删掉（= 保留原值），
 * 客户端干脆不提交，避免出现「空串被当作新密钥」或「清空密钥假成功」。
 */
function withSecrets(patch: JsonObject, secrets: Record<string, string>): JsonObject {
  let result = patch;
  for (const [dottedPath, value] of Object.entries(secrets)) {
    if (value === "") {
      continue;
    }
    const segments = splitPath(dottedPath);
    if (segments.length === 0) {
      continue;
    }
    result = writePath(result, segments, value);
  }
  return result;
}

export function ConfigDraftProvider({ children }: { children: ReactNode }): ReactElement {
  const [baseline, setBaseline] = useState<ConfigRoot | null>(null);
  const [draft, setDraft] = useState<JsonObject>({});
  const [revision, setRevision] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [lastResult, setLastResult] = useState<ConfigSaveResult | null>(null);
  const [secretValues, setSecretValues] = useState<Record<string, string>>({});

  const reload = useCallback(async (): Promise<boolean> => {
    setLoading(true);
    setError(null);
    setWarning(null);
    try {
      const data = await getConfig();
      setBaseline(data);
      setDraft(toDraft(data));
      setRevision(typeof data.revision === "string" ? data.revision : null);
      setConflict(false);
      setLastResult(null);
      // 新的 GET 已把密钥重新打码；旧的明文不能残留，否则下次保存会误覆盖。
      setSecretValues({});
      return true;
    } catch (err: unknown) {
      setError(configErrorMessage(err));
      return false;
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const setField = useCallback((dottedPath: string, value: JsonValue): void => {
    const segments = splitPath(dottedPath);
    if (segments.length === 0) {
      return;
    }
    setDraft((previous) => writePath(previous, segments, value));
  }, []);

  const getField = useCallback(
    (dottedPath: string): JsonValue | undefined =>
      readPath(draft, splitPath(dottedPath)),
    [draft],
  );

  const setSecret = useCallback((dottedPath: string, value: string): void => {
    const normalized = splitPath(dottedPath).join(".");
    if (normalized === "") {
      return;
    }
    setSecretValues((previous) => {
      const next: Record<string, string> = { ...previous };
      if (value === "") {
        // 空串 = 用户清空输入框 = 不提交该密钥（保留服务端原值）。
        delete next[normalized];
      } else {
        next[normalized] = value;
      }
      return next;
    });
  }, []);

  const getSecret = useCallback(
    (dottedPath: string): string | undefined => {
      const normalized = splitPath(dottedPath).join(".");
      return normalized === "" ? undefined : secretValues[normalized];
    },
    [secretValues],
  );

  const save = useCallback(async (): Promise<ConfigSaveResult | null> => {
    if (baseline === null || revision === null) {
      return null;
    }
    const draftPatch = diffPatch(baseline, draft);
    const secretCount = Object.keys(secretValues).length;
    if (Object.keys(draftPatch).length === 0 && secretCount === 0) {
      return null;
    }
    // 密钥明文展开成嵌套路径后再提交；扁平 key 会被服务端 merge_patch 当成真实字段。
    const patch = withSecrets(draftPatch, secretValues);
    // 闭包快照：请求在途期间用户可能继续输入，回来后要能判断 draft / 密钥是否已被改过。
    const submittedDraft = draft;
    const submittedSecrets = secretValues;
    setSaving(true);
    setError(null);
    setWarning(null);
    try {
      const result = await putConfig(patch, revision);
      // 保存成功后**必须回读**：GET 会重新做密钥打码（`has_*` presence 才准）、
      // 服务端也会 normalize 并丢弃「空串密钥」这类不生效的写法。直接拿本地 draft
      // 当 baseline 会让「清空密钥」显示成功而实际未清除、`has_*` 永远停在旧值。
      let fresh: ConfigRoot | null = null;
      try {
        fresh = await getConfig();
      } catch (readbackErr: unknown) {
        setWarning(
          `配置已保存，但重新读取失败（${configErrorMessage(readbackErr)}）。页面上密钥「已配置」状态可能过期，请手动刷新。`,
        );
      }
      const nextRevision =
        typeof fresh?.revision === "string"
          ? fresh.revision
          : typeof result.revision === "string"
            ? result.revision
            : revision;
      const authoritative: ConfigRoot =
        fresh ?? { ...cloneObject(submittedDraft), revision: nextRevision };
      setBaseline(authoritative);
      setRevision(nextRevision);
      // 只在 draft 未被并发修改时才重置；否则保留用户新输入，dirtyCount 会继续显示新增改动。
      setDraft((current) =>
        current === submittedDraft ? toDraft(authoritative) : current,
      );
      setConflict(false);
      setLastResult(result);
      // 只清掉本次已提交的密钥：保存期间用户新输入的明文不能被旧快照一并抹掉。
      // 失败路径（409 / 其它错误）完全不动 secretValues，用户可直接重试。
      setSecretValues((current) => {
        const next: Record<string, string> = { ...current };
        for (const [path, value] of Object.entries(submittedSecrets)) {
          if (next[path] === value) {
            delete next[path];
          }
        }
        return next;
      });
      return result;
    } catch (err: unknown) {
      if (err instanceof ConfigConflictError) {
        // 冲突：保留 draft 与密钥明文，等用户决定是否重载（不自动覆盖本地改动）。
        // 冲突文案由页面的固定条承担，这里不再写进 error，否则会与冲突条重复、
        // 且后续非 409 错误会被页面的「冲突时隐藏 error」逻辑一起吞掉。
        setConflict(true);
        setLastResult(null);
        return null;
      }
      setError(configErrorMessage(err));
      // 上一次成功的 hot/message 必须清掉，否则底部仍显示上一次的「立即生效」。
      setLastResult(null);
      return null;
    } finally {
      setSaving(false);
    }
  }, [baseline, draft, revision, secretValues]);

  const discard = useCallback((): void => {
    if (baseline === null) {
      return;
    }
    setDraft(toDraft(baseline));
    setSecretValues({});
    setConflict(false);
    setError(null);
  }, [baseline]);

  const resolveConflict = useCallback(async (): Promise<void> => {
    await reload();
  }, [reload]);

  const dirtyCount = useMemo(
    () =>
      (baseline === null ? 0 : countDirty(draft, baseline)) +
      Object.keys(secretValues).length,
    [baseline, draft, secretValues],
  );

  const dirtyPaths = useMemo(() => {
    const patch: JsonObject = baseline === null ? {} : diffPatch(baseline, draft);
    return [...dirtyPathsOf(patch), ...Object.keys(secretValues)];
  }, [baseline, draft, secretValues]);

  const value = useMemo<ConfigDraftValue>(
    () => ({
      baseline,
      draft,
      revision,
      saving,
      loading,
      error,
      warning,
      conflict,
      lastResult,
      secretValues,
      dirtyCount,
      dirtyPaths,
      setField,
      getField,
      setSecret,
      getSecret,
      save,
      discard,
      reload,
      resolveConflict,
    }),
    [
      baseline,
      draft,
      revision,
      saving,
      loading,
      error,
      warning,
      conflict,
      lastResult,
      secretValues,
      dirtyCount,
      dirtyPaths,
      setField,
      getField,
      setSecret,
      getSecret,
      save,
      discard,
      reload,
      resolveConflict,
    ],
  );

  return (
    <ConfigDraftContext.Provider value={value}>{children}</ConfigDraftContext.Provider>
  );
}

/** 读取配置草稿状态；必须在 `ConfigDraftProvider` 内使用。 */
export function useConfigDraft(): ConfigDraftValue {
  const value = useContext(ConfigDraftContext);
  if (value === null) {
    throw new Error("useConfigDraft 必须在 ConfigDraftProvider 内使用");
  }
  return value;
}
