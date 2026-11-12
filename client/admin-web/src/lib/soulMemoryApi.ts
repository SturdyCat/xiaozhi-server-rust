/**
 * 人格 / 记忆段（W8 / Config C5）的端点封装：`[soul]` 预设与预览、
 * `[memory]` 状态 / 测试召回 / 维护 / 清空 / RAG 重建。
 *
 * 契约以服务端源码为准，**与 W7（nluApi.ts）相反的三点**（头注释必须写明）：
 * 1. 7 个端点**全部** `#[cfg(feature = "native-plugins")]` 门控挂载
 *    （`server/src/gateway/ws/mod.rs:254-270`）——pure-base 构建下请求落到 SPA
 *    fallback 返回 **HTML 200**（`requestApi` 抛 `SyntaxError`）。
 * 2. 全部 handler **无鉴权**（不调 `auth_ok`，无 `?token=` 依赖）：本文件**不写**
 *    任何 401 / token 分支；`requestApi` 自动带的 Bearer 头无害。
 * 3. 请求体全是裸 `serde_json::Value`，**无 `deny_unknown_fields`**：多余字段被
 *    静默忽略，不必像 W7 那样逐键裁剪请求体。
 *
 * 但与 W7 相同的两条纪律不变：
 * - **全部判业务形状，HTTP 200 不算成功**：`ok !== true` 或关键字段缺失一律按
 *   失败处理；形状不符（含 HTML 200 的 `JSON.parse` 失败）抛
 *   [`SoulMemoryUnavailableError`]，UI 层用 `UnavailableNotice` 呈现，
 *   **禁止伪装成空数据 / 空目录 / 加载成功**。
 * - 错误形状统一 `{ok:false,message}`（`soul.rs:96-102` / `memory.rs:194-208`，
 *   400 也是 JSON）：错误归一读响应体 `message`，风格对齐 nluApi 的
 *   `normalizeError`（不吞异常、不伪装成功）。
 *
 * 路径写法沿用 `configApi.ts` 惯例：只传相对 `requestApi` 的路径，不拼 `/api` 前缀。
 */

import { ApiError, apiGet, apiPost } from "@/lib/api";
import type { JsonObject, JsonValue } from "@/lib/configApi";

/** 人格 / 记忆能力未启用（native-plugins 未编入，SPA fallback 接走）的可识别错误。 */
export class SoulMemoryUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SoulMemoryUnavailableError";
  }
}

/** 供页面判断是否改用 `UnavailableNotice` 呈现。 */
export function isSoulMemoryUnavailable(error: unknown): boolean {
  return error instanceof SoulMemoryUnavailableError;
}

/** 把任意异常转成可展示的中文文案。 */
export function soulMemoryErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

/** 判断是否为普通 JSON 对象（数组与 null 不算）。 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 尽力从错误响应体取 `{ok:false,message}` 的 message；纯文本兜底。 */
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
    }
  } catch {
    // 非 JSON 错误体直接回落原文。
  }
  return text;
}

const FALLBACK_HINT =
  "服务端返回了非 JSON 内容（通常是该构建未启用 native-plugins，请求被管理页 HTML 回退接走）。";

/**
 * 错误归一：这里**没有 401 / token 分支**（7 个端点全部无鉴权）；400 / 500 都是
 * JSON `{ok:false,message}`，读 message 透出；`SyntaxError` 归一成能力未启用。
 */
function normalizeError(error: unknown, action: string): never {
  if (error instanceof SoulMemoryUnavailableError) {
    throw error;
  }
  if (error instanceof ApiError) {
    const detail = readServerMessage(error.body) ?? `HTTP ${error.status}`;
    throw new ApiError(`${action}失败：${detail}`, error.status, error.body);
  }
  if (error instanceof SyntaxError) {
    throw new SoulMemoryUnavailableError(`${action}：${FALLBACK_HINT}`);
  }
  if (error instanceof Error) {
    throw error;
  }
  throw new Error(String(error));
}

/** 形状守卫：`ok !== true` 或不是对象时抛出（HTML 200 / 缺字段都不能伪装成功）。 */
function requireOk(raw: unknown, action: string): Record<string, unknown> {
  if (!isRecord(raw)) {
    throw new SoulMemoryUnavailableError(
      `${action}：响应不是 JSON 对象，按能力未启用处理（不伪装成空数据）。${FALLBACK_HINT}`,
    );
  }
  if (raw["ok"] !== true) {
    const detail =
      typeof raw["message"] === "string" && raw["message"].trim() !== ""
        ? raw["message"].trim()
        : "服务端未给出失败原因";
    throw new Error(`${action}失败：${detail}`);
  }
  return raw;
}

/** 安全读取字符串字段；缺失/类型不符返回 null。 */
function str(raw: Record<string, unknown>, key: string): string | null {
  const value = raw[key];
  return typeof value === "string" && value !== "" ? value : null;
}

/** 安全读取字符串数组。 */
function strArray(value: unknown): string[] {
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

/** 安全读取数字字段；缺失/非有限数返回 null。 */
function num(raw: Record<string, unknown>, key: string): number | null {
  const value = raw[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

// ============================================================
// 1. GET /api/soul/presets
// ============================================================

/** 内置人格预设条目（`presets::json_list()`；`profile` 可原样喂给表单）。 */
export interface SoulPreset {
  id: string;
  name: string;
  description: string;
  /** 完整 SoulConfig JSON；只读展示，载入走 preview 的 effective。 */
  profile: JsonValue;
}

/** `GET /api/soul/presets` 归一后的响应。 */
export interface SoulPresets {
  presets: SoulPreset[];
  defaultId: string | null;
  noneId: string | null;
  note: string | null;
}

/**
 * 读取内置人格预设清单。必须判 `ok === true && presets 是数组`，不以 200 判成功；
 * pure-base 下被 SPA fallback 接走 → [`SoulMemoryUnavailableError`]。
 */
export async function getSoulPresets(): Promise<SoulPresets> {
  const raw = await apiGet<unknown>("soul/presets", {
    action: "读取人格预设清单",
    onError: normalizeError,
  });
  const body = requireOk(raw, "读取人格预设清单");
  if (!Array.isArray(body["presets"])) {
    throw new SoulMemoryUnavailableError(
      "读取人格预设清单：响应缺少 presets 数组，按能力未启用处理（不伪装成空清单）。",
    );
  }
  const presets: SoulPreset[] = [];
  for (const item of body["presets"]) {
    if (!isRecord(item)) {
      continue;
    }
    presets.push({
      id: typeof item["id"] === "string" ? item["id"] : "",
      name: typeof item["name"] === "string" ? item["name"] : "",
      description: typeof item["description"] === "string" ? item["description"] : "",
      profile: (item["profile"] ?? null) as JsonValue,
    });
  }
  return {
    presets: presets.filter((preset) => preset.id !== ""),
    defaultId: str(body, "default_id"),
    noneId: str(body, "none_id"),
    note: str(body, "note"),
  };
}

// ============================================================
// 2. POST /api/soul/preview
// ============================================================

/** `POST /api/soul/preview` 归一后的响应（预览**只读**，不落盘）。 */
export interface SoulPreview {
  instructions: string;
  chars: number | null;
  /** 由内置预设补上的字段名（你留空的字段）。 */
  fromPreset: string[];
  /** 你自己填了值的字段名。 */
  overridden: string[];
  /** 服务端合并后的完整档案（「载入预设内容到表单」的数据源）。 */
  effective: JsonObject | null;
  approxTokens: number | null;
  enabled: boolean | null;
  presetName: string | null;
  /** `"soul"` = 来自人格档案；其他值 = 人格未启用（用 llm.system_prompt）。 */
  source: string | null;
  /** 服务端说明原文（禁用时是「人格未启用：…」），必须原样透出。 */
  note: string | null;
}

/**
 * 预览最终提示词。`soul` 传当前草稿的 `[soul]` 对象（未保存也能预览）；传 null
 * 时省略该键，服务端回落盘上配置。请求体是裸 Value（多余字段被忽略）。
 */
export async function previewSoul(soul: JsonObject | null): Promise<SoulPreview> {
  const payload: Record<string, JsonValue> = {};
  if (soul !== null) {
    payload["soul"] = soul;
  }
  const raw = await apiPost<unknown>("soul/preview", payload, {
    action: "预览最终提示词",
    onError: normalizeError,
  });
  const body = requireOk(raw, "预览最终提示词");
  const instructions = typeof body["instructions"] === "string" ? body["instructions"] : "";
  return {
    instructions,
    chars: num(body, "chars"),
    fromPreset: strArray(body["from_preset"]),
    overridden: strArray(body["overridden"]),
    effective: isRecord(body["effective"]) ? (body["effective"] as JsonObject) : null,
    approxTokens: num(body, "approx_tokens"),
    enabled: typeof body["enabled"] === "boolean" ? body["enabled"] : null,
    presetName: str(body, "preset_name"),
    source: str(body, "source"),
    note: str(body, "note"),
  };
}

// ============================================================
// 3. GET /api/memory/status
// ============================================================

/** 单个作用域的条数（`types.rs:175`）。 */
export interface MemoryScopeStat {
  scope: string;
  memories: number;
}

/** RAG 子状态（只在 rag 后端时出现在响应里，`types.rs:180-183`）。 */
export interface MemoryRagStats {
  docs: number;
  chunks: number;
  building: boolean;
  lastBuildAtMs: number | null;
  lastBuildDocs: number | null;
  lastBuildChunks: number | null;
  lastError: string | null;
}

/** `GET /api/memory/status` 归一后的状态卡（只读投影）。 */
export interface MemoryStatus {
  engine: string;
  active: boolean | null;
  dbPath: string | null;
  dbBytes: number | null;
  turns: number | null;
  messages: number | null;
  memories: number | null;
  pending: number | null;
  quarantined: number | null;
  stored: number | null;
  scopes: MemoryScopeStat[];
  /** true = 无 embedding：当前为本地词法召回（不是故障，要如实说明）。 */
  lexicalOnly: boolean | null;
  /** 抽取器摘要（unknown 透传）。 */
  extractor: JsonValue;
  /** 服务端塞进来的配置摘要（unknown 透传）。 */
  config: JsonValue;
  /** 能力阶段 + 降级原因（`memory.rs:53` capability_json）。 */
  capabilityPhase: string | null;
  capabilityLastError: string | null;
  /** AIUI 全链路模式下的显式提示（云端闭环时人格/记忆不生效）。 */
  aiuiNote: string | null;
  /** rag 段**按存在性判断**：graph/store 后端没有该键（不是 0 值）。 */
  rag: MemoryRagStats | null;
}

/**
 * 读取记忆状态卡。必须判 `ok === true`；形状不符按能力未启用处理。
 * `rag` 键**按存在性判断**（`types.rs:180-183`：graph/store 的 JSON 没有 rag 段）。
 */
export async function getMemoryStatus(): Promise<MemoryStatus> {
  const raw = await apiGet<unknown>("memory/status", {
    action: "读取记忆状态",
    onError: normalizeError,
  });
  const body = requireOk(raw, "读取记忆状态");
  const capability = isRecord(body["capability"]) ? body["capability"] : null;
  const ragRaw = isRecord(body["rag"]) ? body["rag"] : null;
  const scopesRaw = Array.isArray(body["scopes"]) ? body["scopes"] : [];
  const scopes: MemoryScopeStat[] = [];
  for (const item of scopesRaw) {
    if (isRecord(item) && typeof item["scope"] === "string") {
      scopes.push({
        scope: item["scope"],
        memories: typeof item["memories"] === "number" ? item["memories"] : 0,
      });
    }
  }
  return {
    engine: typeof body["engine"] === "string" ? body["engine"] : "",
    active: typeof body["active"] === "boolean" ? body["active"] : null,
    dbPath: str(body, "db_path"),
    dbBytes: num(body, "db_bytes"),
    turns: num(body, "turns"),
    messages: num(body, "messages"),
    memories: num(body, "memories"),
    pending: num(body, "pending"),
    quarantined: num(body, "quarantined"),
    stored: num(body, "stored"),
    scopes,
    lexicalOnly: typeof body["lexical_only"] === "boolean" ? body["lexical_only"] : null,
    extractor: (body["extractor"] ?? null) as JsonValue,
    config: (body["config"] ?? null) as JsonValue,
    capabilityPhase: capability === null ? null : str(capability, "phase"),
    capabilityLastError: capability === null ? null : str(capability, "last_error"),
    aiuiNote: str(body, "aiui_note"),
    rag:
      ragRaw === null
        ? null
        : {
            docs: typeof ragRaw["docs"] === "number" ? ragRaw["docs"] : 0,
            chunks: typeof ragRaw["chunks"] === "number" ? ragRaw["chunks"] : 0,
            building: ragRaw["building"] === true,
            lastBuildAtMs:
              typeof ragRaw["last_build_at_ms"] === "number" ? ragRaw["last_build_at_ms"] : null,
            lastBuildDocs:
              typeof ragRaw["last_build_docs"] === "number" ? ragRaw["last_build_docs"] : null,
            lastBuildChunks:
              typeof ragRaw["last_build_chunks"] === "number" ? ragRaw["last_build_chunks"] : null,
            lastError: str(ragRaw, "last_error"),
          },
  };
}

// ============================================================
// 4. POST /api/memory/recall
// ============================================================

/** `POST /api/memory/recall` 归一后的结果（wire 形状以 `types.rs:23-33` 为准）。 */
export interface MemoryRecallResult {
  /** 命中条数（服务端 `hits` 是 **usize 数字**，不是数组；`types.rs:25`）。 */
  hitCount: number | null;
  /** 命中词项**计数**（服务端 `matched_terms` 是 usize 数字，不是词项列表；`types.rs:26`）。 */
  matchedTermCount: number | null;
  elapsedMs: number | null;
  /** 是否把正文注入了 system prompt（服务端 `injected = block.is_some()`，布尔）。 */
  injected: boolean | null;
  /** 注入正文（服务端 `preview = block`，Option<String>；作为背景资料，不是指令）。 */
  preview: string | null;
  /** 服务端错误信息（如抽取/召回内部降级），有值必须透出。 */
  error: string | null;
  scope: string | null;
  note: string | null;
}

/**
 * 按一句话试召回。`text` 必填（服务端空串 400，前端空串不发请求）；`scope`
 * 可省（省略 = 跨全部作用域）。请求体是裸 Value。
 */
export async function recallMemory(body: {
  text: string;
  scope?: string;
}): Promise<MemoryRecallResult> {
  const payload: Record<string, string> = { text: body.text };
  if (body.scope !== undefined && body.scope.trim() !== "") {
    payload["scope"] = body.scope.trim();
  }
  const raw = await apiPost<unknown>("memory/recall", payload, {
    action: "测试召回",
    onError: normalizeError,
  });
  const res = requireOk(raw, "测试召回");
  return {
    // 服务端 wire：hits / matched_terms 都是 usize 数字（types.rs:23-33），不存在
    // hit_count 键；照「hits 是数组」的臆测解析会把有数据读成 null（W8 缺陷单 major）。
    hitCount: num(res, "hits"),
    matchedTermCount: num(res, "matched_terms"),
    elapsedMs: num(res, "elapsed_ms"),
    injected: typeof res["injected"] === "boolean" ? res["injected"] : null,
    preview: str(res, "preview"),
    error: str(res, "error"),
    scope: str(res, "scope"),
    note: str(res, "note"),
  };
}

// ============================================================
// 5. POST /api/memory/maintain
// ============================================================

/** `POST /api/memory/maintain` 归一后的结果。 */
export interface MemoryMaintainResult {
  ran: boolean | null;
  released: number | null;
  deleted: number | null;
  /** 与请求一致：true = 演练（未删除），文案必须如实区分。 */
  dryRun: boolean;
}

/**
 * 立即维护（保留策略 + 隔离释放）。前端**显式传全** `{force:true, dry_run}`，
 * 不依赖服务端默认值；`dry_run=true` 是演练（未删除），文案不得说成已清理。
 */
export async function maintainMemory(dryRun: boolean): Promise<MemoryMaintainResult> {
  const action = dryRun ? "演练保留策略" : "立即维护";
  const raw = await apiPost<unknown>(
    "memory/maintain",
    { force: true, dry_run: dryRun },
    { action, onError: normalizeError },
  );
  const res = requireOk(raw, action);
  return {
    ran: typeof res["ran"] === "boolean" ? res["ran"] : null,
    released: num(res, "released"),
    deleted: num(res, "deleted"),
    dryRun: res["dry_run"] === true,
  };
}

// ============================================================
// 6. POST /api/memory/clear
// ============================================================

/** `POST /api/memory/clear` 归一后的结果。 */
export interface MemoryClearResult {
  deleted: number | null;
  scope: string | null;
  all: boolean;
  /** 服务端确认文案（「已清空 N 条轮次记忆…」），原样透出。 */
  message: string | null;
}

/**
 * 清空记忆。`{scope}` 或 `{all:true}` **强制显式**（两者皆无服务端 400）；
 * 两段式确认由 UI 层实现，本封装不做确认。
 */
export async function clearMemory(body: {
  scope?: string;
  all?: boolean;
}): Promise<MemoryClearResult> {
  const payload: Record<string, JsonValue> = {};
  if (body.all === true) {
    payload["all"] = true;
  } else if (body.scope !== undefined && body.scope.trim() !== "") {
    payload["scope"] = body.scope.trim();
  }
  const raw = await apiPost<unknown>("memory/clear", payload, {
    action: "清空记忆",
    onError: normalizeError,
  });
  const res = requireOk(raw, "清空记忆");
  return {
    deleted: num(res, "deleted"),
    scope: str(res, "scope"),
    all: res["all"] === true,
    message: str(res, "message"),
  };
}

// ============================================================
// 7. POST /api/memory/rag/reindex
// ============================================================

/** `POST /api/memory/rag/reindex` 归一后的结果（异步后台任务，无完成语义）。 */
export interface MemoryReindexResult {
  building: boolean;
  /**
   * 服务端 message **必须原样透出**：正常是「已开始后台重建索引…」，防重入时
   * 是「索引已在重建中（本次触发被防重入挡下）…」——两者都 ok:true，
   * **绝不能因 ok:true 就显示「已完成/成功重建」**。
   */
  message: string;
}

/**
 * 触发 RAG 文档索引后台重建。异步任务：立即返回 `building:true`，进度查
 * `/api/memory/status` 的 rag 段（前端不做自动轮询，提示手动刷新）。
 */
export async function reindexRag(): Promise<MemoryReindexResult> {
  const raw = await apiPost<unknown>(
    "memory/rag/reindex",
    {},
    {
      action: "重建文档索引",
      onError: normalizeError,
    },
  );
  const res = requireOk(raw, "重建文档索引");
  if (typeof res["building"] !== "boolean") {
    throw new Error(
      "重建文档索引：响应缺少 building 布尔字段，不能判定为已受理（不伪装成已开始）。",
    );
  }
  return {
    building: res["building"],
    message:
      typeof res["message"] === "string" && res["message"] !== ""
        ? res["message"]
        : "服务端未返回说明文案；进度请查 /api/memory/status 的 rag 段。",
  };
}
