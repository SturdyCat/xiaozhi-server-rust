/**
 * 配置域 API 客户端（`/api/config` 系列）。
 *
 * 契约以服务端源码为准：
 * - `server/src/gateway/ws/config.rs`（GET / PUT）
 * - `server/src/gateway/ws/config/save.rs`（写库与 409）
 * - `plugins/registry/projection.rs`（`GET /api/config/schema` 投影）
 *
 * 关键语义：
 * - GET 返回**整份配置对象** + 包装字段 `revision`（内容指纹），密钥字段已被摘除，
 *   只留 `has_<field>` 布尔 presence。
 * - PUT 是**增量合并**：出现即覆盖，未出现保持现盘值；显式 `null` 表示清除该字段。
 * - 并发字段 `expected_revision`（服务端也接受 `revision`）；成功响应里的
 *   `hot` / `hot_hint` / `changed_sections` 是**扁平字段**（服务端 `json_msg_full`
 *   实际写法，见 config.rs 的 `json_msg_full`），并非嵌套的 `impact` 对象。
 *   这里同时保留可选的 `impact` 形状以兼容声明型契约，并提供
 *   [`saveImpactOf`] 把两种形状归一。
 */

import { ApiError, apiGet, apiPost, apiPut } from "@/lib/api";

/** JSON 标量。 */
export type JsonPrimitive = string | number | boolean | null;

/** 任意 JSON 值（配置树只允许 JSON 可表示的值）。 */
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

/** JSON 对象。 */
export interface JsonObject {
  [key: string]: JsonValue;
}

/**
 * `GET /api/config` 响应：整份 Config 对象 + 包装字段 `revision`。
 * `revision` 是内容指纹（不含密钥明文），保存时原样回传用于乐观并发控制。
 */
export interface ConfigRoot extends JsonObject {
  revision: string;
}

/** 保存后的生效档位（服务端注册表字段元数据的唯一权威结论）。 */
export type ConfigHotLevel = "live" | "next_session" | "restart_only";

/** 一次保存的生效影响（声明型形状；实际 wire 见下方 `ConfigSaveResult`）。 */
export interface ConfigSaveImpact {
  hot: ConfigHotLevel;
  sections: string[];
  paths: string[];
}

/**
 * `PUT /api/config` 成功响应。
 *
 * 服务端实际返回扁平字段 `hot` / `hot_hint` / `changed_sections`
 * （`json_msg_full`），不返回 `paths`；`impact` 为可选的兼容形状。
 */
export interface ConfigSaveResult {
  ok: boolean;
  message?: string;
  config_path?: string | null;
  revision?: string | null;
  hot?: ConfigHotLevel;
  hot_hint?: string | null;
  changed_sections?: string[];
  impact?: ConfigSaveImpact;
}

/** `GET /api/config/presets` 的 wire 是 `string[]`；这里转成对象便于后续加字段。 */
export interface PresetInfo {
  name: string;
}

/** 单个字段的元数据（`GET /api/config/schema`）。 */
export interface ConfigFieldInfo {
  key: string;
  label: string;
  kind: string;
  secret: boolean;
  default_hint: string;
  help: string;
  hot: ConfigHotLevel;
  hot_hint: string;
  required: boolean;
  advanced: boolean;
  /** 仅 `kind === "enum"` 时存在。 */
  options?: string[];
}

/** 公共字段分组：`fields_path` 是字段所在配置路径（如 `["tts"]`）。 */
export interface ConfigFieldGroup {
  fields_path: string[];
  fields: ConfigFieldInfo[];
}

/** 一个能力实现（如 `tts.kokoro`）。 */
export interface ConfigImplementationInfo {
  id: string;
  display: string;
  local: boolean;
  hot: ConfigHotLevel;
  hot_hint: string;
  requires: string[];
  fields_path: string[];
  fields: ConfigFieldInfo[];
}

/** 一个能力（ASR / TTS / LLM / ...）的字段元数据。 */
export interface ConfigCapabilityInfo {
  id: string;
  display: string;
  required: boolean;
  implemented: boolean;
  common_fields: ConfigFieldGroup[];
  implementations: ConfigImplementationInfo[];
}

/** 非插件配置段（`[server]` / `[audio]` 等）的生效档位。 */
export interface ConfigSectionHot {
  id: string;
  hot: ConfigHotLevel;
  hot_hint: string;
}

/** `GET /api/config/schema` 响应体（手写投影，snake_case 对齐 wire）。 */
export interface ConfigSchemaInfo {
  capabilities: ConfigCapabilityInfo[];
  /** 高级字段的完整点分路径清单（`tts.kokoro.num_threads` 这类）。 */
  advanced_fields: string[];
  app_sections: ConfigSectionHot[];
}

/** `GET /api/config/meta` 响应体：配置文件路径与持久化判断。 */
export interface ConfigMetaInfo {
  config_path: string | null;
  backend: string | null;
  loaded_from: string | null;
  persistent: boolean | null;
  hint: string;
}

/** 409 并发冲突：保留服务端回传的当前 revision，便于提示“重新加载”。 */
export class ConfigConflictError extends ApiError {
  public readonly currentRevision: string | null;
  public readonly conflictMessage: string;

  constructor(status: number, body: string, currentRevision: string | null) {
    super(`配置保存冲突：HTTP ${status}`, status, body);
    this.name = "ConfigConflictError";
    this.currentRevision = currentRevision;
    this.conflictMessage = extractMessage(body) ?? "配置已被其他窗口/端修改，请重新加载后再改。";
  }
}

/** 从响应体里尽力取出服务端的中文 `message`（兼容 JSON 与纯文本）。 */
function extractMessage(body: string): string | null {
  const text = body.trim();
  if (text === "") {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(text);
    if (typeof parsed === "object" && parsed !== null && "message" in parsed) {
      const message = (parsed as { message?: unknown }).message;
      if (typeof message === "string" && message.trim() !== "") {
        return message;
      }
    }
  } catch {
    // 400 合并失败是纯文本，直接回落下面的文本分支。
  }
  return text;
}

/** 把任意异常转成可展示的中文消息；优先使用服务端 `message`。 */
export function configErrorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    return extractMessage(error.body) ?? error.message;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

/** 把保存响应归一成 `{hot,sections,paths}`；无档位信息时返回 null。 */
export function saveImpactOf(result: ConfigSaveResult): ConfigSaveImpact | null {
  if (result.impact) {
    return result.impact;
  }
  if (result.hot) {
    return {
      hot: result.hot,
      sections: result.changed_sections ?? [],
      // 服务端未在 wire 上暴露 paths（只有日志里有）。
      paths: [],
    };
  }
  return null;
}

/** 拉取整份配置（含 revision）。 */
export const getConfig = () => apiGet<ConfigRoot>("config");

/**
 * 增量保存配置。
 *
 * `expectedRevision` 非空时写入 `expected_revision`；服务端指纹不符会返回 409，
 * 这里把它转成 [`ConfigConflictError`]（带 `currentRevision`），**不吞掉**。
 */
export async function putConfig(
  patch: JsonObject,
  expectedRevision?: string,
): Promise<ConfigSaveResult> {
  const payload: JsonObject =
    expectedRevision === undefined ? patch : { ...patch, expected_revision: expectedRevision };
  return apiPut<ConfigSaveResult>("config", payload, {
    // 业务覆盖范本：409 → ConfigConflictError（带 currentRevision），其余异常原样上抛。
    onError: (error) => {
      if (error instanceof ApiError && error.status === 409) {
        throw new ConfigConflictError(error.status, error.body, readRevision(error.body));
      }
      throw error;
    },
  });
}

/** 从 409 响应体里读取当前 revision。 */
function readRevision(body: string): string | null {
  try {
    const parsed: unknown = JSON.parse(body);
    if (typeof parsed === "object" && parsed !== null && "revision" in parsed) {
      const revision = (parsed as { revision?: unknown }).revision;
      if (typeof revision === "string") {
        return revision;
      }
    }
  } catch {
    // 非 JSON 响应体：保持 null。
  }
  return null;
}

/** 拉取字段元数据（默认值 / 热生效档位 / 密钥标记 / 高级字段清单）。 */
export const getConfigSchema = () => apiGet<ConfigSchemaInfo>("config/schema");

/** 拉取配置文件来源元信息（路径 / 后端 / 是否持久化）。 */
export const getConfigMeta = () => apiGet<ConfigMetaInfo>("config/meta");

/** 列出全部预设名。 */
export async function listPresets(): Promise<PresetInfo[]> {
  const result = await apiGet<{ presets?: string[] }>("config/presets");
  return (result.presets ?? []).map((name) => ({ name }));
}

/** 把当前生效配置保存为预设。 */
export function savePreset(name: string): Promise<ConfigSaveResult> {
  return apiPost<ConfigSaveResult>("config/presets", { name });
}

/** 应用预设到全局配置（复用服务端部分更新管线）。 */
export function applyPreset(name: string): Promise<ConfigSaveResult> {
  return apiPost<ConfigSaveResult>(`config/presets/${encodeURIComponent(name)}/apply`);
}

/** 删除预设。管理台统一走 POST 别名（服务端同时保留 DELETE 路由）。 */
export function deletePreset(name: string): Promise<ConfigSaveResult> {
  return apiPost<ConfigSaveResult>(`config/presets/${encodeURIComponent(name)}/delete`);
}
