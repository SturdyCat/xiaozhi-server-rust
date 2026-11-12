/**
 * 配置表单的「在线目录 / 凭据自检」端点封装（W6 / Config C3）。
 *
 * 契约以服务端源码为准：
 * - `server/src/gateway/ws/bench_api.rs:175-206`（`POST /api/test/llm/models`，
 *   `ModelsRequest` 带 `deny_unknown_fields`）、`server/src/benchmark/models.rs:4-32`
 *   （`ModelsResult`：HTTP 200 也可能是 `state:"error"`）。
 * - `server/src/device/voices.rs:36-48`（`Voice`）、`:240-254`（GET 目录）、
 *   `:256-287`（POST refresh，400 纯文本 / 401 纯文本）、`:289-346`（POST test-credentials，
 *   200 也可能是 `{ok:false,error}`）、`:349-354`（路由挂载）。
 * - `server/src/gateway/ws/mod.rs:244-250`：`/api/tts/voices*` 仅在
 *   `#[cfg(feature = "native-plugins")]` 下挂载；未编入时请求会被
 *   `:327-331` 的 SPA fallback（`ServeDir` → `index.html`）接走，返回 **HTML 200**。
 *
 * 因此所有端点都不以 HTTP 200 作为成功判据：必须判业务字段（`state === "ok"` /
 * `ok === true` / 响应形状）。特别是 `getTtsVoices()` 走 **fail-closed**：
 * 响应不是「含 voices 数组 + probing 布尔的 JSON 对象」时抛
 * [`CatalogueUnavailableError`]，由页面用 `UnavailableNotice` 呈现，绝不伪装成
 * 空目录、绝不伪装成加载成功、绝不把 SyntaxError 当成普通错误糊过去。
 *
 * 路径写法沿用 `configApi.ts` 惯例：只传相对 `requestApi` 的路径，不拼 `/api` 前缀。
 */

import { ApiError, apiGet, apiPost } from "@/lib/api";

/** `POST /api/test/llm/models` 请求体：只允许这两个键，且只带非空值（可只发 `{}`）。 */
export interface LlmModelsRequest {
  api_base?: string;
  api_key?: string;
}

/** `POST /api/tts/voices/test-credentials` 请求体：空/缺字段由服务端逐项回落盘上凭据。 */
export interface TtsCredentialsRequest {
  app_id?: string;
  api_key?: string;
  api_secret?: string;
  voice?: string;
}

/** 目录条目（`server/src/device/voices.rs:36-48`），按 gender × type 分组用。 */
export interface Voice {
  vcn: string;
  name: string;
  gender: string;
  type: string;
  tag: string;
}

/** `GET /api/tts/voices` 归一后的响应。 */
export interface VoicesResult {
  voices: Voice[];
  /** `"probe"` = 实测结果；`"empty"` + `fetchedAt === 0` = 尚未探测。 */
  source: string;
  fetchedAt: number;
  probing: boolean;
}

/** 获取在线模型列表成功的结果。 */
export interface LlmModelsResult {
  models: string[];
  elapsedMs: number | null;
}

/** `POST /api/tts/voices/refresh` 成功结果（已判 `ok === true`）。 */
export interface RefreshResult {
  ok: true;
  probing: boolean;
  candidates: number;
}

/** `POST /api/tts/voices/test-credentials` 的两种业务结果（只测不存）。 */
export type CredentialsTestResult = { ok: true; elapsedMs: number } | { ok: false; error: string };

/** 音色目录能力未启用 / 响应被 SPA fallback 接走的可识别错误。 */
export class CatalogueUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "CatalogueUnavailableError";
  }
}

/** 供页面判断是否改用 `UnavailableNotice` 呈现。 */
export function isCatalogueUnavailable(error: unknown): boolean {
  return error instanceof CatalogueUnavailableError;
}

/** 把任意异常转成可展示的中文文案（401 已在 [`normalizeError`] 里明确成「需要管理鉴权」）。 */
export function catalogueErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

/** 判断是否为普通 JSON 对象（数组与 null 不算）。 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 尽力从响应体取服务端文案：兼容 JSON（message / error）与 400/401 纯文本。 */
function readServerMessage(body: string): string | null {
  const text = body.trim();
  if (text === "") {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(text);
    if (isRecord(parsed)) {
      const message = parsed["message"];
      if (typeof message === "string" && message.trim() !== "") {
        return message.trim();
      }
      const error = parsed["error"];
      if (typeof error === "string" && error.trim() !== "") {
        return error.trim();
      }
    }
  } catch {
    // 400 未配置三要素 / 401 unauthorized 都是纯文本，直接回落原文。
  }
  return text;
}

const FALLBACK_HINT =
  "服务端返回了非 JSON 内容（通常是该构建未启用 native-plugins，请求被管理页 HTML 回退接走）。";

/**
 * 把底层异常归一成「可展示、不吞 401、不伪装成功」的错误后抛出。
 *
 * - `ApiError 401` → 明确「需要管理鉴权」，保留 401 语义；
 * - `ApiError 404/501` → 能力未启用；
 * - 其余 `ApiError` → 透出服务端可读文案（纯文本兜底）；
 * - `SyntaxError`（`requestApi` 里 `JSON.parse` 解析 HTML 200）→ 能力未启用。
 */
function normalizeError(error: unknown, action: string): never {
  if (error instanceof CatalogueUnavailableError) {
    throw error;
  }
  if (error instanceof ApiError) {
    if (error.status === 401) {
      throw new ApiError(
        `需要管理鉴权：${action}被服务端拒绝（HTTP 401）。请在地址栏附带 ?token=<管理令牌> 后重试。`,
        401,
        error.body,
      );
    }
    if (error.status === 404 || error.status === 501) {
      throw new CatalogueUnavailableError(
        `${action}：服务端返回 HTTP ${error.status}，音色目录能力未启用或端点未挂载。`,
      );
    }
    const detail = readServerMessage(error.body);
    throw new ApiError(
      `${action}失败：${detail ?? `HTTP ${error.status}`}`,
      error.status,
      error.body,
    );
  }
  if (error instanceof SyntaxError) {
    throw new CatalogueUnavailableError(`${action}：${FALLBACK_HINT}`);
  }
  if (error instanceof Error) {
    throw error;
  }
  throw new Error(String(error));
}

/**
 * 拉取上游 OpenAI 兼容端点的模型名列表（只测不存）。
 *
 * `ModelsRequest` 带 `deny_unknown_fields`，因此 body 只含非空 `api_base` / `api_key`，
 * 全空时发 `{}`（服务端回落盘上已存 `[llm]` 凭据）。HTTP 200 也可能
 * `state:"error"`，必须判 `state === "ok"` 才取 `models`。
 */
export async function fetchLlmModels(body: LlmModelsRequest): Promise<LlmModelsResult> {
  const payload: Record<string, string> = {};
  if (body.api_base !== undefined && body.api_base.trim() !== "") {
    payload.api_base = body.api_base.trim();
  }
  if (body.api_key !== undefined && body.api_key.trim() !== "") {
    payload.api_key = body.api_key.trim();
  }

  const raw = await apiPost<unknown>("test/llm/models", payload, {
    action: "获取模型列表",
    onError: normalizeError,
  });

  if (!isRecord(raw) || typeof raw.state !== "string") {
    throw new Error(`获取模型列表：服务端响应缺少 state 字段，不能判定为成功。${FALLBACK_HINT}`);
  }
  if (raw.state !== "ok") {
    const detail =
      typeof raw.error === "string" && raw.error.trim() !== ""
        ? raw.error.trim()
        : `state=${raw.state}`;
    throw new Error(`获取模型列表失败：${detail}`);
  }
  if (!Array.isArray(raw.models)) {
    throw new Error("获取模型列表：state=ok 但 models 不是数组，不能伪装成空列表。");
  }
  const models: string[] = [];
  for (const item of raw.models) {
    if (typeof item === "string" && item.trim() !== "") {
      models.push(item.trim());
    }
  }
  const elapsedMs =
    typeof raw.elapsed_ms === "number" && Number.isFinite(raw.elapsed_ms) ? raw.elapsed_ms : null;
  return { models, elapsedMs };
}

/** 把未知形状的目录条目收敛成 `Voice`；缺字段按空串处理，不抛异常。 */
function toVoice(value: unknown): Voice {
  if (!isRecord(value)) {
    return { vcn: "", name: "", gender: "", type: "", tag: "" };
  }
  const str = (key: string): string => {
    const item = value[key];
    return typeof item === "string" ? item : "";
  };
  return {
    vcn: str("vcn"),
    name: str("name"),
    gender: str("gender"),
    type: str("type"),
    tag: str("tag"),
  };
}

/**
 * 读取音色目录（fail-closed）。
 *
 * 未编入 `native-plugins` 时 `/api/tts/voices` 被 SPA fallback 接走返回 HTML 200，
 * `requestApi` 的 `JSON.parse` 会抛 `SyntaxError` → 归一成
 * [`CatalogueUnavailableError`]；即便拿到 JSON，也必须是「对象 + voices 数组 +
 * probing 布尔」，否则同样按能力未启用处理。
 */
export async function getTtsVoices(): Promise<VoicesResult> {
  const raw = await apiGet<unknown>("tts/voices", {
    action: "读取音色目录",
    onError: normalizeError,
  });

  if (!isRecord(raw) || !Array.isArray(raw.voices) || typeof raw.probing !== "boolean") {
    throw new CatalogueUnavailableError(
      `读取音色目录：响应不是含 voices 数组与 probing 字段的 JSON 对象，按能力未启用处理（不伪装成空目录）。${FALLBACK_HINT}`,
    );
  }

  const voices: Voice[] = [];
  for (const item of raw.voices) {
    const voice = toVoice(item);
    if (voice.vcn !== "") {
      voices.push(voice);
    }
  }
  const fetchedAt =
    typeof raw.fetched_at === "number" && Number.isFinite(raw.fetched_at) ? raw.fetched_at : 0;
  return {
    voices,
    source: typeof raw.source === "string" ? raw.source : "",
    fetchedAt,
    probing: raw.probing,
  };
}

/**
 * 触发服务端后台真实探测（立即 200 返回 `probing:true`，前端随后轮询目录）。
 *
 * 未配置讯飞三要素时服务端返回 **400 纯文本**（非 JSON），401 是纯文本
 * `unauthorized`，两者都由 [`normalizeError`] / [`readServerMessage`] 兜底。
 * 必须判 `ok === true`，不能只看 HTTP 200。
 */
export async function refreshTtsVoices(): Promise<RefreshResult> {
  const raw = await apiPost<unknown>(
    "tts/voices/refresh",
    {},
    {
      action: "启动音色探测",
      onError: normalizeError,
    },
  );

  if (!isRecord(raw) || raw.ok !== true || typeof raw.probing !== "boolean") {
    throw new Error(
      `启动音色探测：服务端响应缺少 ok=true / probing 字段，不能判定为已启动。${FALLBACK_HINT}`,
    );
  }
  const candidates =
    typeof raw.candidates === "number" && Number.isFinite(raw.candidates) ? raw.candidates : 0;
  return { ok: true, probing: raw.probing, candidates };
}

/**
 * 用表单里的讯飞三要素真实合成一次（**只测不存**，服务端逐项回落盘上凭据）。
 *
 * 空/缺字段不发送，前端可只带改动项。HTTP 200 也可能是 `{ok:false,error:"…"}`，
 * 必须判 `ok` 后返回结果；失败结果里保留 `error`（含 11200 等可行动提示）供页面展示。
 */
export async function testTtsCredentials(
  body: TtsCredentialsRequest,
): Promise<CredentialsTestResult> {
  const payload: Record<string, string> = {};
  for (const key of ["app_id", "api_key", "api_secret", "voice"] as const) {
    const value = body[key];
    if (typeof value === "string" && value.trim() !== "") {
      payload[key] = value.trim();
    }
  }

  const raw = await apiPost<unknown>("tts/voices/test-credentials", payload, {
    action: "测试讯飞凭据",
    onError: normalizeError,
  });

  if (!isRecord(raw) || typeof raw.ok !== "boolean") {
    throw new Error(
      `测试讯飞凭据：服务端响应缺少布尔字段 ok，不能判定为测试成功。${FALLBACK_HINT}`,
    );
  }
  if (raw.ok === false) {
    const detail =
      typeof raw.error === "string" && raw.error.trim() !== ""
        ? raw.error.trim()
        : "服务端未给出失败原因";
    return { ok: false, error: detail };
  }
  const elapsedMs =
    typeof raw.elapsed_ms === "number" && Number.isFinite(raw.elapsed_ms) ? raw.elapsed_ms : 0;
  return { ok: true, elapsedMs };
}
