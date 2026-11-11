/**
 * NLU 段（W7 / Config C4）的端点封装：模型全生命周期 + 只读技能目录。
 *
 * 契约以服务端源码为准：
 * - `server/src/gateway/ws/nlu_build.rs`：`GET/POST /api/nlu/model/build`（状态 / 启动，
 *   `BuildRequest` 带 `deny_unknown_fields`，body 只能含 `expected_revision`）、
 *   错误形状统一 `{state:"error",error_code,error}`（`:86-92`）、`auto_activated` 恒为
 *   false（`:356` 硬编码，构建永不自动启用）。
 * - `server/src/gateway/ws/nlu_build/history.rs:44-60`：`GET /api/nlu/model/builds`
 *   返回 `{builds,total,active_build_id}`；根目录不存在返回空列表（正常，不是 404）。
 * - `server/src/gateway/ws/nlu_model.rs:60-67`：`POST /api/nlu/model/test`
 *   （`TestRequest` 带 `deny_unknown_fields`：只收 `build_id` / `text` /
 *   `expected_intent`），响应 `measurement === "real_onnx_inference"` 且
 *   `executed === false`；服务端自持 45s 超时，前端另设 60s 本地超时兜底。
 * - `server/src/gateway/ws/skill_api.rs:21-28,107-139`：`GET /api/skills/catalog`
 *   返回 `{state:"ok",revision,recognition,intents,effective,capabilities,...}`；
 *   该路由随 `skill_api::routes()` 挂载，pure-base 构建下请求会被 SPA fallback
 *   接走返回 **HTML 200**（`requestApi` 抛 `SyntaxError`）——`getSkillCatalog()`
 * 走 fail-closed：形状不符抛 [`NluUnavailableError`]，绝不伪装成空目录。
 *
 * 路径写法沿用 `configApi.ts` / `catalogueApi.ts` 惯例：只传相对 `requestApi`
 * 的路径，不拼 `/api` 前缀。所有端点都不以 HTTP 200 作为成功判据。
 */

import { ApiError, requestApi } from "@/lib/api";
import type { JsonObject, JsonValue } from "@/lib/configApi";

/** 技能控制域白名单，与服务端 `Domain` 枚举一一对应（`plugins/nlu/mod.rs:136-146`）。 */
export const NLU_DOMAINS = ["server", "device", "iot"] as const;

/** 构建状态四态（`nlu_build.rs:53-67` 的 `state` 字段）。 */
export type NluBuildPhase = "idle" | "running" | "success" | "failed";

/** `GET /api/nlu/model/build` 归一后的状态。 */
export interface NluBuildStatus {
  phase: string;
  running: boolean;
  /** 最近一次失败原因（服务端 `error` 字段），无失败时为 null。 */
  error: string | null;
  /** 尾部日志（服务端截断 8192 字符，`nlu_build.rs:125-138`），非全量增量。 */
  logTail: string;
  buildId: string | null;
  sourceRevision: string | null;
  outputDir: string | null;
  origin: string | null;
  /** 原始 artifact 对象（含 build_id / source_revision / model_sha256 / expected_config）。 */
  artifact: JsonObject | null;
  /** artifact.expected_config：写回草稿的 bert 四字段来源。 */
  expectedConfig: JsonObject | null;
  /** 服务端硬编码恒为 false：构建成功**永不**自动启用（`nlu_build.rs:356`）。 */
  autoActivated: boolean;
}

/** `POST /api/nlu/model/build` 成功结果（已判 `started === true`）。 */
export interface NluBuildStartResult {
  started: true;
  buildId: string;
  /** 受理时回显的保存态版本；与服务端不一致时启动响应会被判失败。 */
  sourceRevision: string;
}

/** 构建历史行（`history.rs:50-59`）。 */
export interface NluBuildHistoryRow {
  buildId: string;
  /** ISO 时间或 null；null = 历史 manifest 未记录，**绝不猜成当前底座**。 */
  createdAt: string | null;
  /** 训练底座规范名；null/缺失 = 历史构建未记录。 */
  baseModel: string | null;
  labels: string[];
  /** 四态：ok / unverified / incomplete / corrupt（`artifact.rs:128-131`）。 */
  integrity: string;
  missing: string[];
  active: boolean;
  /** corrupt 行的失败原因（目录在但与记录对不上），正常行为 null。 */
  error: string | null;
}

/** `GET /api/nlu/model/builds` 归一后的历史。 */
export interface NluBuildHistory {
  builds: NluBuildHistoryRow[];
  total: number;
  activeBuildId: string | null;
}

/** `POST /api/nlu/model/test` 请求体：`deny_unknown_fields`，只允许这三个键。 */
export interface NluTestRequest {
  buildId: string;
  text: string;
  /** 期望技能名；服务端可选，但 UI 层必填（verified 判定依赖 expected_match）。 */
  expectedIntent: string;
}

/** `POST /api/nlu/model/test` 归一后的结果（候选测试**不写任何配置**）。 */
export interface NluTestResult {
  buildId: string;
  modelSha256: string;
  sourceRevision: string;
  rawLabel: string;
  /** 映射后的技能名（未映射时与 rawLabel 一致或为 other）。 */
  intent: string;
  confidence: number;
  threshold: number;
  accepted: boolean;
  rejectReason: string;
  expectedIntent: string | null;
  /** 仅请求带 expected_intent 时非 null：预测是否与期望一致。 */
  expectedMatch: boolean | null;
  tokenizeMs: number;
  inferMs: number;
}

/** 技能目录条目（`skill_api.rs:110-129`，含 `source` 与能力 metadata）。 */
export interface SkillCatalogIntent {
  name: string;
  domain: string;
  templates: string[];
  /** 原 action 对象（kind / reply / plan 等），只读展示。 */
  action: JsonValue;
  /** command / default / custom（`skill_api.rs:114-124`）。 */
  source: string;
  /** 该技能可用的服务端能力名清单（可能缺省）。 */
  capabilities: string[];
}

/** `GET /api/skills/catalog` 归一后的保存态目录（只读，与编辑器并存）。 */
export interface SkillCatalog {
  revision: string;
  recognition: string;
  intents: SkillCatalogIntent[];
}

/** 技能目录能力未启用 / 响应被 SPA fallback 接走的可识别错误。 */
export class NluUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "NluUnavailableError";
  }
}

/** 供页面判断是否改用 `UnavailableNotice` 呈现。 */
export function isNluUnavailable(error: unknown): boolean {
  return error instanceof NluUnavailableError;
}

/** 把任意异常转成可展示的中文文案。 */
export function nluErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

/** 判断是否为普通 JSON 对象（数组与 null 不算）。 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 尽力从错误响应体取服务端文案与 error_code（JSON 或纯文本兜底）。 */
function readErrorBody(body: string): { code: string | null; message: string | null } {
  const text = body.trim();
  if (text === "") {
    return { code: null, message: null };
  }
  try {
    const parsed: unknown = JSON.parse(text);
    if (isRecord(parsed)) {
      const code =
        typeof parsed["error_code"] === "string" && parsed["error_code"] !== ""
          ? parsed["error_code"]
          : null;
      const message =
        typeof parsed["error"] === "string" && parsed["error"].trim() !== ""
          ? parsed["error"].trim()
          : typeof parsed["message"] === "string" && parsed["message"].trim() !== ""
            ? parsed["message"].trim()
            : null;
      return { code, message };
    }
  } catch {
    // 401 的纯文本 `unauthorized` 等直接回落原文。
  }
  return { code: null, message: text };
}

const FALLBACK_HINT =
  "服务端返回了非 JSON 内容（通常是该构建未启用对应能力，请求被管理页 HTML 回退接走）。";

/**
 * 把底层异常归一成「可展示、不吞 401、不伪装成功」的错误后抛出。
 *
 * - `ApiError 401` → 明确「需要管理鉴权」；
 * - `ApiError 503` → 按 error_code 区分 dev 推理禁用 / nlu feature 未启用；
 * - `ApiError 504` → 超时不代表失败；
 * - `ApiError 429` → 诊断/目录忙；
 * - `ApiError 409` → 按 error_code 区分 build_running / revision_conflict；
 * - `SyntaxError`（SPA fallback HTML 200）→ [`NluUnavailableError`]。
 */
function normalizeError(error: unknown, action: string): never {
  if (error instanceof NluUnavailableError) {
    throw error;
  }
  if (error instanceof ApiError) {
    const { code, message } = readErrorBody(error.body);
    const detail = message ?? `HTTP ${error.status}`;
    if (error.status === 401) {
      throw new ApiError(
        `需要管理鉴权：${action}被服务端拒绝（HTTP 401）。请在地址栏附带 ?token=<管理令牌> 后重试。`,
        401,
        error.body,
      );
    }
    if (error.status === 503) {
      if (code === "dev_inference_disabled") {
        throw new Error(`推理端点在 dev 模式禁用（${action}）：请用生产构建的服务端再试。`);
      }
      if (code === "nlu_feature_disabled") {
        throw new Error(
          `${action}：服务端未编译 nlu feature（--features nlu），无法进行真实候选推理。`,
        );
      }
      throw new Error(`${action}失败：服务端返回 503（${detail}）。`);
    }
    if (error.status === 504) {
      throw new Error(
        `${action}超时（HTTP 504）。超时不代表失败：服务端可能仍在执行，请稍后刷新状态再试。`,
      );
    }
    if (error.status === 429) {
      throw new Error(
        `${action}被限流（HTTP 429）：已有同类操作在执行（诊断忙 / 目录忙），请稍后再试。`,
      );
    }
    if (error.status === 409) {
      if (code === "build_running") {
        throw new Error(`${action}冲突：已有一个模型构建在运行（唯一槽位）。请等待其结束后再试。`);
      }
      if (code === "revision_conflict") {
        throw new Error(
          `${action}冲突：保存态配置已变化（revision_conflict）。请重新加载配置后再试。`,
        );
      }
      throw new Error(`${action}冲突（HTTP 409）：${detail}`);
    }
    throw new ApiError(`${action}失败：${detail}`, error.status, error.body);
  }
  if (error instanceof SyntaxError) {
    throw new NluUnavailableError(`${action}：${FALLBACK_HINT}`);
  }
  if (error instanceof DOMException && error.name === "AbortError") {
    throw new Error(`${action}本地超时（60 秒）。超时不代表失败：服务端可能仍在推理，请稍后重试。`);
  }
  if (error instanceof Error) {
    throw error;
  }
  throw new Error(String(error));
}

/** 判定构建响应的业务错误形状 `{state:"error",error_code,error}` 并抛出。 */
function throwNluBusinessError(raw: Record<string, unknown>, action: string): never {
  const detail =
    typeof raw["error"] === "string" && raw["error"].trim() !== ""
      ? raw["error"].trim()
      : "服务端未给出失败原因";
  throw new Error(`${action}失败：${detail}`);
}

/** 从原始 artifact 对象安全读取字符串字段；缺失返回 null。 */
function artifactString(artifact: JsonObject, key: string): string | null {
  const value = artifact[key];
  return typeof value === "string" && value !== "" ? value : null;
}

/**
 * 读取模型构建状态（`GET /api/nlu/model/build`）。
 *
 * 必须判业务形状：`type === "state"` 且 `state` 是字符串；`auto_activated`
 * 服务端硬编码 false，这里仍做强校验，防止「构建成功自动启用」的假象。
 */
export async function getNluBuildStatus(): Promise<NluBuildStatus> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("nlu/model/build");
  } catch (error) {
    normalizeError(error, "读取模型构建状态");
  }
  if (
    !isRecord(raw) ||
    raw["type"] !== "state" ||
    typeof raw["state"] !== "string" ||
    typeof raw["running"] !== "boolean"
  ) {
    throw new Error(
      `读取模型构建状态：响应不是含 type=state 的 JSON 对象，不能判定为成功。${FALLBACK_HINT}`,
    );
  }
  const artifact = isRecord(raw["artifact"]) ? (raw["artifact"] as JsonObject) : null;
  const expectedConfig = isRecord(raw["expected_config"])
    ? (raw["expected_config"] as JsonObject)
    : null;
  return {
    phase: raw["state"],
    running: raw["running"],
    error: typeof raw["error"] === "string" && raw["error"] !== "" ? raw["error"] : null,
    logTail: typeof raw["log_tail"] === "string" ? raw["log_tail"] : "",
    buildId:
      (artifact !== null ? artifactString(artifact, "build_id") : null) ??
      (typeof raw["build_id"] === "string" && raw["build_id"] !== "" ? raw["build_id"] : null),
    sourceRevision:
      (artifact !== null ? artifactString(artifact, "source_revision") : null) ??
      (typeof raw["source_revision"] === "string" && raw["source_revision"] !== ""
        ? raw["source_revision"]
        : null),
    outputDir:
      typeof raw["output_dir"] === "string" && raw["output_dir"] !== "" ? raw["output_dir"] : null,
    origin: typeof raw["origin"] === "string" ? raw["origin"] : null,
    artifact,
    expectedConfig,
    autoActivated: raw["auto_activated"] === true,
  };
}

/**
 * 启动一次模型构建（`POST /api/nlu/model/build`）。
 *
 * `BuildRequest` 带 `deny_unknown_fields`（`nlu_build.rs:182-186`），body **只发**
 * `{expected_revision}`；不传 `base_model`（多传直接 400）。必须判
 * `started === true && state === "running"` 且 `build_id` / `source_revision` 在场，
 * `source_revision` 还要回显校验与请求一致。
 */
export async function startNluBuild(expectedRevision: string): Promise<NluBuildStartResult> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("nlu/model/build", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ expected_revision: expectedRevision }),
    });
  } catch (error) {
    normalizeError(error, "启动模型构建");
  }
  if (!isRecord(raw)) {
    throw new Error(`启动模型构建：响应不是 JSON 对象，不能判定为已启动。${FALLBACK_HINT}`);
  }
  if (
    raw["state"] === "error" ||
    (typeof raw["error_code"] === "string" && raw["error_code"] !== "")
  ) {
    throwNluBusinessError(raw, "启动模型构建");
  }
  const buildId =
    typeof raw["build_id"] === "string" && raw["build_id"] !== "" ? raw["build_id"] : null;
  const sourceRevision =
    typeof raw["source_revision"] === "string" && raw["source_revision"] !== ""
      ? raw["source_revision"]
      : null;
  if (
    raw["started"] !== true ||
    raw["state"] !== "running" ||
    buildId === null ||
    sourceRevision === null
  ) {
    throw new Error(
      "启动模型构建：服务端响应缺少 started=true / state=running / build_id / source_revision，不能判定为已启动。",
    );
  }
  if (sourceRevision !== expectedRevision) {
    throw new Error(
      `启动模型构建：受理回显的保存态版本（${sourceRevision}）与请求（${expectedRevision}）不一致，按失败处理。`,
    );
  }
  return { started: true, buildId, sourceRevision };
}

/** 安全读取历史行的字符串数组字段。 */
function stringArrayOf(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const items: string[] = [];
  for (const item of value) {
    if (typeof item === "string") {
      items.push(item);
    }
  }
  return items;
}

/**
 * 读取构建历史（`GET /api/nlu/model/builds`）。
 *
 * 根目录不存在时服务端返回空列表（正常回答）；必须判 `builds` 是数组，
 * 不以 HTTP 200 判成功。
 */
export async function getNluBuildHistory(): Promise<NluBuildHistory> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("nlu/model/builds");
  } catch (error) {
    normalizeError(error, "读取构建历史");
  }
  if (!isRecord(raw) || !Array.isArray(raw["builds"])) {
    throw new Error(
      `读取构建历史：响应不是含 builds 数组的 JSON 对象，不能判定为成功。${FALLBACK_HINT}`,
    );
  }
  const builds: NluBuildHistoryRow[] = [];
  for (const item of raw["builds"]) {
    if (!isRecord(item)) {
      continue;
    }
    builds.push({
      buildId: typeof item["build_id"] === "string" ? item["build_id"] : "",
      createdAt:
        typeof item["created_at"] === "string" && item["created_at"] !== ""
          ? item["created_at"]
          : null,
      baseModel:
        typeof item["base_model"] === "string" && item["base_model"] !== ""
          ? item["base_model"]
          : null,
      labels: stringArrayOf(item["labels"]),
      integrity: typeof item["integrity"] === "string" ? item["integrity"] : "",
      missing: stringArrayOf(item["missing"]),
      active: item["active"] === true,
      error: typeof item["error"] === "string" && item["error"] !== "" ? item["error"] : null,
    });
  }
  const total =
    typeof raw["total"] === "number" && Number.isFinite(raw["total"])
      ? raw["total"]
      : builds.length;
  const activeBuildId =
    typeof raw["active_build_id"] === "string" && raw["active_build_id"] !== ""
      ? raw["active_build_id"]
      : null;
  return { builds, total, activeBuildId };
}

/**
 * 用候选模型做一次真实 ONNX 推理（`POST /api/nlu/model/test`，**只测不写**）。
 *
 * `TestRequest` 带 `deny_unknown_fields`（`nlu_model.rs:21-27`），body 只发三个键。
 * 响应强校验：`state === "ok"` 且 `measurement === "real_onnx_inference"` 且
 * `executed === false`（`nlu_model.rs:61-66`）；build_id / model_sha256 /
 * source_revision 的**一致性**由页面拿着候选信息比对（`ConfigNluModelState.kt:188-192`
 * 同款口径）。本地 60s 超时兜底（服务端自持 45s）。
 */
export async function testNluModel(body: NluTestRequest): Promise<NluTestResult> {
  const payload: Record<string, string> = {
    build_id: body.buildId,
    text: body.text,
    expected_intent: body.expectedIntent,
  };
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("nlu/model/test", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
      // 本地 60s 超时兜底；服务端 45s 自持超时，正常应先返回。
      signal: AbortSignal.timeout(60_000),
    });
  } catch (error) {
    normalizeError(error, "候选模型测试");
  }
  if (!isRecord(raw)) {
    throw new Error(`候选模型测试：响应不是 JSON 对象，不能判定为成功。${FALLBACK_HINT}`);
  }
  if (raw["state"] === "error") {
    throwNluBusinessError(raw, "候选模型测试");
  }
  if (
    raw["state"] !== "ok" ||
    raw["measurement"] !== "real_onnx_inference" ||
    raw["executed"] !== false
  ) {
    throw new Error(
      "候选模型测试：响应缺少 state=ok / measurement=real_onnx_inference / executed=false 的推理证据，不能采信。",
    );
  }
  const number = (key: string): number =>
    typeof raw[key] === "number" && Number.isFinite(raw[key]) ? raw[key] : 0;
  return {
    buildId: typeof raw["build_id"] === "string" ? raw["build_id"] : "",
    modelSha256: typeof raw["model_sha256"] === "string" ? raw["model_sha256"] : "",
    sourceRevision: typeof raw["source_revision"] === "string" ? raw["source_revision"] : "",
    rawLabel: typeof raw["raw_label"] === "string" ? raw["raw_label"] : "",
    intent: typeof raw["intent"] === "string" ? raw["intent"] : "",
    confidence: number("confidence"),
    threshold: number("threshold"),
    accepted: raw["accepted"] === true,
    rejectReason: typeof raw["reject_reason"] === "string" ? raw["reject_reason"] : "",
    expectedIntent: typeof raw["expected_intent"] === "string" ? raw["expected_intent"] : null,
    expectedMatch:
      raw["expected_match"] === true ? true : raw["expected_match"] === false ? false : null,
    tokenizeMs: number("tokenize_ms"),
    inferMs: number("infer_ms"),
  };
}

/** 安全归一目录条目：缺字段降级为空值，不抛异常、不丢条目。 */
function toCatalogIntent(value: unknown): SkillCatalogIntent {
  if (!isRecord(value)) {
    return {
      name: "",
      domain: "",
      templates: [],
      action: null,
      source: "",
      capabilities: [],
    };
  }
  const name = typeof value["name"] === "string" ? value["name"] : "";
  const domain = typeof value["domain"] === "string" ? value["domain"] : "";
  const templates = stringArrayOf(value["templates"]);
  const source = typeof value["source"] === "string" ? value["source"] : "";
  const capabilities = stringArrayOf(value["capabilities"]);
  const rawAction = value["action"];
  return {
    name,
    domain,
    templates,
    action: rawAction === undefined ? null : (rawAction as JsonValue),
    source,
    capabilities,
  };
}

/**
 * 读取保存态技能目录（`GET /api/skills/catalog`，只读，与编辑器并存）。
 *
 * **fail-closed**：pure-base 构建下请求会被 SPA fallback 接走返回 HTML 200
 * （`requestApi` 抛 `SyntaxError` → [`NluUnavailableError`]）；即便拿到 JSON，
 * 也必须是「对象 + state==="ok" + intents 数组」，否则同样按能力未启用处理，
 * 绝不伪装成空目录、绝不伪装成加载成功。
 */
export async function getSkillCatalog(): Promise<SkillCatalog> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("skills/catalog");
  } catch (error) {
    normalizeError(error, "读取技能目录");
  }
  if (!isRecord(raw) || raw["state"] !== "ok" || !Array.isArray(raw["intents"])) {
    throw new NluUnavailableError(
      `读取技能目录：响应不是含 state="ok" 与 intents 数组的 JSON 对象，按能力未启用处理（不伪装成空目录）。${FALLBACK_HINT}`,
    );
  }
  const intents = raw["intents"].map(toCatalogIntent);
  return {
    revision: typeof raw["revision"] === "string" ? raw["revision"] : "",
    recognition: typeof raw["recognition"] === "string" ? raw["recognition"] : "",
    intents,
  };
}
