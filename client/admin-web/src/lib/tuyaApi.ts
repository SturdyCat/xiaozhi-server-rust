/**
 * 涂鸦设备面（W12 / devicehub）端点封装。
 *
 * 契约权威：`docs/plans/admin-web-migration/w12-devicehub-research.md` 附录 A/C
 * （闸门 G3、错误形状 E1、10 method wire 表），全部经 `server/src/device/tuya/*.rs`
 * 逐行核实。
 *
 * ## 实现范围
 *
 * - 10 method 全实现：status / credentials(POST) / credentials(DELETE) /
 *   credentials/delete(POST 别名) / sync / devices / devices/{id} / dp / alias /
 *   cloud/proxy。其中 `deleteTuyaCredentialsAlias`（POST 别名，Kuikly NetworkModule
 *   无 DELETE 的产物）与 `proxyTuyaCloud` **UI 本波不调用**——浏览器可原生 DELETE、
 *   云代理是高级调试通道（旧端亦无 UI 入口），按 wire 表补齐，头注释登记。
 * - **凭证唯一录入路径是 `POST /api/tuya/credentials`**（C6 铁律）：本文件不接
 *   ConfigDraftContext / setField / PUT /api/config 主链；Config C6 只暴露
 *   enabled/sync_interval_secs，两套 UI 不互相覆盖。
 * - `access_secret` 任何响应/日志**永不回显**（mod.rs 安全铁律）；status 响应
 *   `access_id_masked` 是服务端掩码（`ab****ef`，≤4 位 → `****`）。
 *
 * ## fail-closed 双路径（检查单①；G3 门控路由）
 *
 * pure-base 下整组 `/api/tuya/*` 不挂载 → SPA fallback **HTML 200**。catch 链两条
 * 失败路径缺一不可：① `SyntaxError → TuyaUnavailableError`；② JSON 形状不符 → 归一。
 * **`GET /api/tuya/status` 与 `GET /api/tuya/devices` 响应无 `ok` 键**——判
 * `enabled`/`configured`/`lan_unregistered` 与 `devices[]`/`total`，严禁判 `ok===true`。
 *
 * ## 鉴权清单（检查单②）
 *
 * 全部 10 个 handler 都是 G3（`auth_ok` → `tuya.enabled`）：401 **纯文本**
 * `"unauthorized"`（非 JSON）→ 逐方法写 401 分支（按 `status===401` 区分）；404 JSON
 * `{"error":"tuya disabled"}` → `TuyaUnavailableError(kind="disabled")`。Bearer 由
 * requestApi 自动注入，token 不进 URL。
 *
 * ## sync 语义（诚实化锚点 5）
 *
 * `POST /api/tuya/sync` 回 `{result,message}`：`ok/partial/cloud_auth_failed/
 * network_error/skipped`（服务端实际下发集合；**不存在 "busy"**——并发场景 sync.rs:45
 * 返回 `result="skipped"` + message「已有一次同步在进行中」）。
 * **`skipped`（60s 闸门或并发拦下）不算失败、也不得报
 * 成功**——`resultLabel("skipped")`=「已跳过」，由 UI 以中性语气透出服务端 message。
 */

import { ApiError, requestApi } from "@/lib/api";
import type { JsonValue } from "@/lib/configApi";

/**
 * 涂鸦能力结构性不可用。`kind`：`not-compiled`（HTML 200 / 404 空体）|
 * `disabled`（`{"error":"tuya disabled"}`）。
 */
export class TuyaUnavailableError extends Error {
  readonly kind: "not-compiled" | "disabled";

  constructor(message: string, kind: "not-compiled" | "disabled") {
    super(message);
    this.name = "TuyaUnavailableError";
    this.kind = kind;
  }
}

/** 业务层错误：携带服务端原文与（控制失败时的）kind/advice 中文原文，UI 逐字透出。 */
export class TuyaApiError extends Error {
  readonly status: number;
  readonly serverError: string;
  /** 控制失败三归类：offline | timeout | unsupported（lan.rs:67-73）；其余为 null。 */
  readonly kind: string | null;
  /** 服务端中文建议（failure_payload `advice`），透出不英文化。 */
  readonly advice: string | null;

  constructor(
    message: string,
    status: number,
    serverError: string,
    kind: string | null,
    advice: string | null,
  ) {
    super(message);
    this.name = "TuyaApiError";
    this.status = status;
    this.serverError = serverError;
    this.kind = kind;
    this.advice = advice;
  }
}

/** 判断是否为普通 JSON 对象（数组与 null 不算）。 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 安全读取字符串字段（缺失/类型不符返回空串）。 */
function str(raw: Record<string, unknown>, key: string): string {
  return typeof raw[key] === "string" ? (raw[key] as string) : "";
}

/** 安全读取数字字段（缺失/非有限数返回 0）。 */
function num(raw: Record<string, unknown>, key: string): number {
  const value = raw[key];
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

/** 解析错误体（E1 纯 `{"error"}`；控制失败扩展 `kind`/`advice`）。 */
function readErrorBody(body: string): { error: string; kind: string | null; advice: string | null } | null {
  const text = body.trim();
  if (text === "") {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(text);
    if (isRecord(parsed) && typeof parsed["error"] === "string") {
      const kind = parsed["kind"];
      const advice = parsed["advice"];
      return {
        error: parsed["error"],
        kind: typeof kind === "string" ? kind : null,
        advice: typeof advice === "string" ? advice : null,
      };
    }
  } catch {
    // 非 JSON 错误体：返回 null 由调用方兜底原文。
  }
  return null;
}

/** G3 门控错误归一（检查单①双路径 + 检查单②401 纯文本分支）；全部方法共用。 */
function gateError(error: unknown, action: string): never {
  if (error instanceof SyntaxError) {
    throw new TuyaUnavailableError(
      `${action}失败：服务端返回了非 JSON 内容（通常是构建未编入 tuya-plugin，请求被管理页 HTML 回退接走）。按能力未启用处理，不伪装成空数据。`,
      "not-compiled",
    );
  }
  if (error instanceof ApiError) {
    if (error.status === 401) {
      throw new TuyaApiError(
        `${action}失败：HTTP 401 unauthorized（管理台令牌无效或缺失）`,
        401,
        "unauthorized",
        null,
        null,
      );
    }
    const parsed = readErrorBody(error.body);
    if (parsed !== null) {
      if (parsed.error === "tuya disabled") {
        throw new TuyaUnavailableError(
          `${action}失败：tuya disabled（能力已编入但 [tuya].enabled=false）。`,
          "disabled",
        );
      }
      throw new TuyaApiError(`${action}失败：${parsed.error}`, error.status, parsed.error, parsed.kind, parsed.advice);
    }
    if (error.body.trim() !== "") {
      throw new TuyaApiError(`${action}失败：${error.body.trim()}`, error.status, error.body.trim(), null, null);
    }
    if (error.status === 404) {
      throw new TuyaUnavailableError(
        `${action}失败：服务端无此端点（HTTP 404，可能未编入 tuya-plugin）。`,
        "not-compiled",
      );
    }
    throw new TuyaApiError(`${action}失败（HTTP ${error.status}）`, error.status, "", null, null);
  }
  throw error instanceof Error ? error : new Error(`${action}失败：${String(error)}`);
}

// ============================================================
// C.1 GET /api/tuya/status（G3；无 ok 键）
// ============================================================

/** 最近一次云同步摘要（`last_sync`；`age_ms` 服务端算好，客户端不依赖本机时钟）。 */
export interface TuyaLastSync {
  finishedAt: string;
  ageMs: number;
  result: string;
  deviceCount: number;
  message: string;
}

/** `GET /api/tuya/status` 归一后的响应（永不含 access_secret）。 */
export interface TuyaStatus {
  enabled: boolean;
  /** 凭证是否已配置（id+secret 都非空才算，config.rs:123-125）。 */
  configured: boolean;
  /** 服务端掩码（`ab****ef`；≤4 位 → `****`）。 */
  accessIdMasked: string;
  region: string;
  syncIntervalSecs: number;
  syncing: boolean;
  lastSync: TuyaLastSync | null;
  /** LAN 扫描发现的未登记设备快照：不纳管、不落库，只如实展示。 */
  lanUnregistered: { deviceId: string; ip: string; ageMs: number }[];
  storePath: string;
}

function parseLastSync(raw: unknown): TuyaLastSync | null {
  if (!isRecord(raw)) {
    return null;
  }
  return {
    finishedAt: str(raw, "finished_at"),
    ageMs: num(raw, "age_ms"),
    result: str(raw, "result"),
    deviceCount: num(raw, "device_count"),
    message: str(raw, "message"),
  };
}

/**
 * 拉取涂鸦状态（G3）。判形状 `enabled`/`configured`/`lan_unregistered[]`，
 * **不判 `ok===true`（响应无 ok 键）也不判 HTTP 200**。enabled=false 时原样返回
 * （UI 呈「未启用」）。
 */
export async function getTuyaStatus(): Promise<TuyaStatus> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("tuya/status");
  } catch (error) {
    gateError(error, "读取涂鸦状态");
  }
  if (
    !isRecord(raw) ||
    typeof raw["enabled"] !== "boolean" ||
    typeof raw["configured"] !== "boolean" ||
    !Array.isArray(raw["lan_unregistered"])
  ) {
    throw new Error("读取涂鸦状态：响应形状不符（缺少 enabled/configured/lan_unregistered）。");
  }
  const lanUnregistered: { deviceId: string; ip: string; ageMs: number }[] = [];
  for (const item of raw["lan_unregistered"]) {
    if (!isRecord(item)) {
      continue;
    }
    lanUnregistered.push({
      deviceId: str(item, "device_id"),
      ip: str(item, "ip"),
      ageMs: num(item, "age_ms"),
    });
  }
  return {
    enabled: raw["enabled"],
    configured: raw["configured"],
    accessIdMasked: str(raw, "access_id_masked"),
    region: str(raw, "region"),
    syncIntervalSecs: num(raw, "sync_interval_secs"),
    syncing: raw["syncing"] === true,
    lastSync: parseLastSync(raw["last_sync"]),
    lanUnregistered,
    storePath: str(raw, "store_path"),
  };
}

// ============================================================
// C.2/C.3/C.4 凭证三路（G3；唯一录入路径 POST /credentials）
// ============================================================

/** 录入/更新涂鸦云凭证（secret 留空且已配置 = 保留旧值）。 */
export interface TuyaCredentialsInput {
  region: string;
  accessId: string;
  /** 留空且已配置 = 保留旧值；服务端校验后永不回显。 */
  accessSecret: string;
}

/**
 * 录入凭证（C.2：`POST /api/tuya/credentials`——**唯一录入路径**，不接 PUT 主配置链）。
 * 成功判 `ok===true`，返回服务端文案「凭证已保存（secret 不再回显）」。
 */
export async function saveTuyaCredentials(input: TuyaCredentialsInput): Promise<string> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("tuya/credentials", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        region: input.region,
        access_id: input.accessId,
        access_secret: input.accessSecret,
      }),
    });
  } catch (error) {
    gateError(error, "保存涂鸦凭证");
  }
  if (!isRecord(raw) || raw["ok"] !== true) {
    throw new Error("保存涂鸦凭证：服务端未确认保存（响应缺少 ok:true）。");
  }
  return str(raw, "message");
}

/**
 * 清除凭证（C.3：DELETE /api/tuya/credentials）。三字段全清，enabled/周期不动。
 * 成功判 `ok===true`，返回服务端文案「凭证已清除」。
 */
export async function clearTuyaCredentials(): Promise<string> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("tuya/credentials", { method: "DELETE" });
  } catch (error) {
    gateError(error, "清除涂鸦凭证");
  }
  if (!isRecord(raw) || raw["ok"] !== true) {
    throw new Error("清除涂鸦凭证：服务端未确认清除（响应缺少 ok:true）。");
  }
  return str(raw, "message");
}

/**
 * 清除凭证的 POST 别名通道（C.4：`POST /api/tuya/credentials/delete`——Kuikly
 * NetworkModule 无 DELETE 的产物，行为同 C.3）。admin-web 浏览器可原生 DELETE
 * （`clearTuyaCredentials`），**UI 不调用本方法**，按 wire 表补齐。
 */
export async function deleteTuyaCredentialsAlias(): Promise<string> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("tuya/credentials/delete", { method: "POST" });
  } catch (error) {
    gateError(error, "清除涂鸦凭证");
  }
  if (!isRecord(raw) || raw["ok"] !== true) {
    throw new Error("清除涂鸦凭证：服务端未确认清除（响应缺少 ok:true）。");
  }
  return str(raw, "message");
}

// ============================================================
// C.5 POST /api/tuya/sync（G3；skipped 不算失败也不报成功）
// ============================================================

/** 手动同步结果（`result` ∈ ok/partial/cloud_auth_failed/network_error/skipped；并发场景 = skipped + message，无 "busy"）。 */
export interface TuyaSyncResult {
  result: string;
  message: string;
}

/** 触发一次云同步（60s 闸门在服务端）。**skipped = 被闸门拦下，不算失败也不报成功**。 */
export async function syncTuyaNow(): Promise<TuyaSyncResult> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("tuya/sync", { method: "POST" });
  } catch (error) {
    gateError(error, "触发涂鸦同步");
  }
  if (!isRecord(raw) || typeof raw["result"] !== "string") {
    throw new Error("触发涂鸦同步：响应形状不符（缺少 result）。");
  }
  return { result: raw["result"], message: str(raw, "message") };
}

/** 同步/控制结果的中文标签（TuyaState.kt:455-463 逐字）。 */
export function tuyaResultLabel(result: string): string {
  switch (result) {
    case "ok":
      return "成功";
    case "partial":
      return "部分成功";
    case "cloud_auth_failed":
      return "凭证失败";
    case "network_error":
      return "网络失败";
    case "skipped":
      return "已跳过";
    default:
      return result;
  }
}

/** 把服务端 `age_ms` 转人话相对时间（服务端算好，客户端不依赖本机时钟）。 */
export function tuyaAgeText(ageMs: number): string {
  if (!Number.isFinite(ageMs) || ageMs < 0) {
    return "—";
  }
  const seconds = Math.floor(ageMs / 1000);
  if (seconds < 60) {
    return `${seconds} 秒前`;
  }
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return `${minutes} 分钟前`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours} 小时前`;
  }
  return `${Math.floor(hours / 24)} 天前`;
}

// ============================================================
// C.6/C.7 设备目录（G3；无 ok 键）
// ============================================================

/** 目录一行（`name` 是 alias 优先于 name——服务端已合并，客户端不再重算）。 */
export interface TuyaDeviceRow {
  deviceId: string;
  name: string;
  model: string;
  online: boolean;
  ip: string;
  updatedAt: string;
}

/** `GET /api/tuya/devices` 归一后的响应（total/online 是过滤后行数统计）。 */
export interface TuyaDevices {
  devices: TuyaDeviceRow[];
  total: number;
  onlineCount: number;
}

/**
 * 拉取设备目录（C.6）。判形状 `devices[]`+`total`，**不判 `ok===true`（无 ok 键）**。
 * `?online=true|false` 可选过滤。
 */
export async function getTuyaDevices(online?: boolean): Promise<TuyaDevices> {
  const query = online === undefined ? "" : `?online=${online ? "true" : "false"}`;
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`tuya/devices${query}`);
  } catch (error) {
    gateError(error, "读取涂鸦设备目录");
  }
  if (!isRecord(raw) || !Array.isArray(raw["devices"]) || typeof raw["total"] !== "number") {
    throw new Error("读取涂鸦设备目录：响应形状不符（缺少 devices/total）。");
  }
  const devices: TuyaDeviceRow[] = [];
  for (const item of raw["devices"]) {
    if (!isRecord(item)) {
      continue;
    }
    devices.push({
      deviceId: str(item, "device_id"),
      name: str(item, "name"),
      model: str(item, "model"),
      online: item["online"] === true,
      ip: str(item, "ip"),
      updatedAt: str(item, "updated_at"),
    });
  }
  return { devices, total: num(raw, "total"), onlineCount: num(raw, "online") };
}

/** 一个数据点（`dps[]` 五字段；`writable`/`control` 由服务端下发，客户端不得重算）。 */
export interface TuyaDp {
  dp: string;
  value: JsonValue;
  /** 服务端生成的中文值文本（bool→开/关；null→空串）。 */
  valueText: string;
  writable: boolean;
  /** boolean→switch / number→number / 其余 readonly。 */
  control: string;
}

/** `GET /api/tuya/devices/{id}` 归一后的响应。 */
export interface TuyaDeviceDetail {
  deviceId: string;
  name: string;
  model: string;
  online: boolean;
  ip: string;
  dps: TuyaDp[];
}

/** 读取单台设备详情（C.7；dps 按 dp 字典序——服务端已排）。 */
export async function getTuyaDevice(deviceId: string): Promise<TuyaDeviceDetail> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`tuya/devices/${encodeURIComponent(deviceId)}`);
  } catch (error) {
    gateError(error, "读取涂鸦设备详情");
  }
  if (!isRecord(raw) || !Array.isArray(raw["dps"])) {
    throw new Error("读取涂鸦设备详情：响应形状不符（缺少 dps 数组）。");
  }
  const dps: TuyaDp[] = [];
  for (const item of raw["dps"]) {
    if (!isRecord(item)) {
      continue;
    }
    dps.push({
      dp: str(item, "dp"),
      value: (item["value"] ?? null) as JsonValue,
      valueText: str(item, "value_text"),
      writable: item["writable"] === true,
      control: str(item, "control") || "readonly",
    });
  }
  return {
    deviceId: str(raw, "device_id"),
    name: str(raw, "name"),
    model: str(raw, "model"),
    online: raw["online"] === true,
    ip: str(raw, "ip"),
    dps,
  };
}

// ============================================================
// C.8/C.9 DP 控制与本地别名（G3）
// ============================================================

/**
 * 下发数据点（C.8：`{"dp","value"}` 两者必填）。成功回 `{dps,message:"已下发"}`。
 * 控制失败按 HTTP 状态三归类（offline→409 / timeout→504 / unsupported→400），错误体
 * `{"error",kind,advice}`——`TuyaApiError.kind`/`advice` 携带中文原文，UI 逐字透出。
 */
export async function setTuyaDp(
  deviceId: string,
  dp: string,
  value: JsonValue,
): Promise<{ message: string }> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`tuya/devices/${encodeURIComponent(deviceId)}/dp`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ dp, value }),
    });
  } catch (error) {
    gateError(error, "下发涂鸦数据点");
  }
  if (!isRecord(raw) || typeof raw["message"] !== "string") {
    throw new Error("下发涂鸦数据点：响应形状不符（缺少 message）。");
  }
  return { message: raw["message"] };
}

/**
 * 保存本地别名（C.9；alias 可空串清除）。**仅本管理台展示，不影响云端名称**。
 * 成功判 `ok===true`。
 */
export async function saveTuyaAlias(deviceId: string, alias: string): Promise<void> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`tuya/devices/${encodeURIComponent(deviceId)}/alias`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ alias }),
    });
  } catch (error) {
    gateError(error, "保存涂鸦本地别名");
  }
  if (!isRecord(raw) || raw["ok"] !== true) {
    throw new Error("保存涂鸦本地别名：服务端未确认保存（响应缺少 ok:true）。");
  }
}

// ============================================================
// C.10 POST /api/tuya/cloud/proxy（G3；UI 本波不调用）
// ============================================================

/** 云代理请求体（method GET/POST/DELETE 白名单；path 必须 `/v1.` 前缀）。 */
export interface TuyaCloudProxyRequest {
  method?: "GET" | "POST" | "DELETE";
  path: string;
  /** query 键按字典序签名（服务端处理）。 */
  query?: Record<string, string>;
  /** 仅 POST 允许。 */
  body?: JsonValue;
}

/**
 * 涂鸦云代理（C.10：服务端签名转发，回 `{status,body}` 原样透传）。
 * **UI 本波不调用**（高级调试通道，旧端亦无 UI 入口），按 wire 表补齐。
 * 502 `{kind:"auth|network|api",error}` → `TuyaApiError`（serverError 透出，消息不含 secret）。
 */
export async function proxyTuyaCloud(
  request: TuyaCloudProxyRequest,
): Promise<{ status: number; body: JsonValue }> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("tuya/cloud/proxy", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        method: request.method ?? "GET",
        path: request.path,
        query: request.query ?? {},
        body: request.body ?? null,
      }),
    });
  } catch (error) {
    gateError(error, "涂鸦云代理请求");
  }
  if (!isRecord(raw) || typeof raw["status"] !== "number") {
    throw new Error("涂鸦云代理请求：响应形状不符（缺少 status）。");
  }
  return { status: raw["status"], body: (raw["body"] ?? null) as JsonValue };
}
