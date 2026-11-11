/**
 * EchoMuse 设备面（W12 / devicehub）端点封装。
 *
 * 契约权威：`docs/plans/admin-web-migration/w12-devicehub-research.md` 附录 A/B
 * （闸门 G0–G4、错误形状 E1–E3、18 端点 wire 表），全部经服务端源码逐行核实。
 *
 * ## 实现范围（R11 拍板）
 *
 * - admin 7 中实现 6（music 本轮不做）；system 5 全做；oww 2 全做；console WS 与
 *   BLE 3 端点本轮不做（六 Tab 详情由 `/em/` iframe 官方页承载，BLE 属延后波次）。
 * - `saveEchomuseSettings` / `setEchomuseVolume` / `getEchomuseSystemLogs` /
 *   `installOwwVad` 四个方法按 wire 实现但 **UI 本波不暴露**（配置编辑、音量、日志、
 *   资产安装由 `/em/` 官方页对应页签承载——旧端注释「官方已有的网络确认与资产安装
 *   不再重复设置入口」），头注释逐个登记。
 *
 * ## fail-closed 双路径（检查单①，本波全部 /api/echomuse/* 都是门控路由）
 *
 * pure-base（`default=[]`，dev-server.sh `--no-default-features`）下整组路由不挂载，
 * 请求落 SPA fallback **HTML 200**。catch 链两条失败路径缺一不可：
 * ① `SyntaxError → EchomuseUnavailableError`（requestApi 对 HTML 200 抛裸 SyntaxError，
 *    api.ts:141；模板 soulMemoryApi.ts:82-114）；
 * ② JSON 解析成功但形状不符 → 归一（清单判 `enabled`+`devices[]`，**响应无 `ok` 键，
 *    严禁判 `ok===true`**）。
 * 特例：`GET /em/status` 编入时**无鉴权且 enabled=false 仍 200 JSON**；未编入时同样落
 * HTML 200 → 与其余端点同一归一。
 *
 * ## 鉴权清单（检查单②，逐 handler）
 *
 * - **G1/G2/G3（401 纯文本 `"unauthorized"`，非 JSON）**：全部 `/api/echomuse/*`——
 *   这些方法**写 401 分支**（按 `status===401` 区分，不当 JSON 解析）；Bearer 由
 *   requestApi 自动注入，token 不进 URL。
 * - **G0 无鉴权**：仅 `GET /em/status`——头注释**显式声明不写 401/token 分支**。
 * - **设备面 `X-Em-Token`**（`[echomuse].token`）：4 个 WS 端点 + `/shell/{id}`——
 *   管理台不直接调（固件拨号），本文件一律不封装。
 * - **两套凭据勿混**：`X-Em-Token` 是设备面（值 `[echomuse].token`）；管理面是
 *   `auth_ok`（`[server].expected_token`/Bearer）。iframe `?token=` 传的是管理台令牌。
 */

import { ApiError, requestApi } from "@/lib/api";
import type { JsonValue } from "@/lib/configApi";

/**
 * EchoMuse 能力结构性不可用。`kind` 供 UI 区分两种诚实文案：
 * - `not-compiled`：构建未编入 echomuse-plugin（HTML 200 被 requestApi 裸抛 / 404 空体）；
 * - `disabled`：已编入但 `[echomuse].enabled=false`（404 `{"error":"echomuse disabled"}`）。
 */
export class EchomuseUnavailableError extends Error {
  readonly kind: "not-compiled" | "disabled";

  constructor(message: string, kind: "not-compiled" | "disabled") {
    super(message);
    this.name = "EchomuseUnavailableError";
    this.kind = kind;
  }
}

/** 业务层错误（401 纯文本 / 4xx JSON / 5xx），携带服务端原文供 UI 逐字透出。 */
export class EchomuseApiError extends Error {
  readonly status: number;
  readonly serverError: string;
  readonly code: string | null;

  constructor(message: string, status: number, serverError: string, code: string | null) {
    super(message);
    this.name = "EchomuseApiError";
    this.status = status;
    this.serverError = serverError;
    this.code = code;
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

function strArray(raw: Record<string, unknown>, key: string): string[] {
  const value = raw[key];
  if (!Array.isArray(value)) {
    return [];
  }
  return value.filter((item): item is string => typeof item === "string");
}

/** 解析错误体（E1 纯 `{"error"}` / E2 `{"error","code"}`；401 是纯文本不走这里）。 */
function readErrorBody(
  body: string,
): { error: string; code: string | null } | null {
  const text = body.trim();
  if (text === "") {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(text);
    if (isRecord(parsed) && typeof parsed["error"] === "string") {
      return {
        error: parsed["error"],
        code: typeof parsed["code"] === "string" ? parsed["code"] : null,
      };
    }
  } catch {
    // 非 JSON 错误体：返回 null 由调用方兜底原文。
  }
  return null;
}

/**
 * 门控端点错误归一（检查单①双路径 + 检查单②401 纯文本分支）。
 * 所有 `/api/echomuse/*` 方法共用；401 是纯文本 `"unauthorized"`，按 status 区分。
 */
function gateError(error: unknown, action: string): never {
  if (error instanceof SyntaxError) {
    throw new EchomuseUnavailableError(
      `${action}失败：服务端返回了非 JSON 内容（通常是构建未编入 echomuse-plugin，请求被管理页 HTML 回退接走）。按能力未启用处理，不伪装成空数据。`,
      "not-compiled",
    );
  }
  if (error instanceof ApiError) {
    if (error.status === 401) {
      throw new EchomuseApiError(
        `${action}失败：HTTP 401 unauthorized（管理台令牌无效或缺失）`,
        401,
        "unauthorized",
        null,
      );
    }
    const parsed = readErrorBody(error.body);
    if (parsed !== null) {
      if (parsed.error === "echomuse disabled") {
        throw new EchomuseUnavailableError(
          `${action}失败：echomuse disabled（能力已编入但 [echomuse].enabled=false）。`,
          "disabled",
        );
      }
      throw new EchomuseApiError(`${action}失败：${parsed.error}`, error.status, parsed.error, parsed.code);
    }
    if (error.body.trim() !== "") {
      throw new EchomuseApiError(`${action}失败：${error.body.trim()}`, error.status, error.body.trim(), null);
    }
    if (error.status === 404) {
      throw new EchomuseUnavailableError(
        `${action}失败：服务端无此端点（HTTP 404，可能未编入 echomuse-plugin）。`,
        "not-compiled",
      );
    }
    throw new EchomuseApiError(`${action}失败（HTTP ${error.status}）`, error.status, "", null);
  }
  throw error instanceof Error ? error : new Error(`${action}失败：${String(error)}`);
}

// ============================================================
// 数据模型（registry.rs record_payload + admin.rs 注入 session_id）
// ============================================================

/** 一台 EchoMuse 设备（`record_payload`，无 `ok` 键）。 */
export interface EchomuseDevice {
  deviceId: string;
  label: string;
  approved: boolean;
  version: string;
  board: string;
  baseOs: string;
  capabilities: string[];
  firstSeen: string;
  lastSeen: string;
  online: boolean;
  connectedSecs: number;
  /** 设备上报的 settings 快照（compat 页签只读展示用）。 */
  settings: Record<string, JsonValue>;
  ip: string;
  kernelArch: string;
  kernelRelease: string;
  bootId: string;
  bootReason: string;
  /** 在线时是会话 UUID 字符串；离线为 null。 */
  sessionId: string | null;
}

function parseDevice(raw: Record<string, unknown>): EchomuseDevice {
  const sessionId = raw["session_id"];
  return {
    deviceId: str(raw, "device_id"),
    label: str(raw, "label"),
    approved: raw["approved"] === true,
    version: str(raw, "version"),
    board: str(raw, "board"),
    baseOs: str(raw, "base_os"),
    capabilities: strArray(raw, "capabilities"),
    firstSeen: str(raw, "first_seen"),
    lastSeen: str(raw, "last_seen"),
    online: raw["online"] === true,
    connectedSecs: num(raw, "connected_secs"),
    settings: isRecord(raw["settings"]) ? (raw["settings"] as Record<string, JsonValue>) : {},
    ip: str(raw, "ip"),
    kernelArch: str(raw, "kernel_arch"),
    kernelRelease: str(raw, "kernel_release"),
    bootId: str(raw, "boot_id"),
    bootReason: str(raw, "boot_reason"),
    sessionId: typeof sessionId === "string" ? sessionId : null,
  };
}

// ============================================================
// B.1 GET /api/echomuse/devices（G1；无 ok 键）
// ============================================================

/** `GET /api/echomuse/devices` 归一后的响应。 */
export interface EchomuseDevices {
  /** enabled=false 时 UI 呈「未启用」而不是空列表。 */
  enabled: boolean;
  /** 设备面变化号（与 WS `echomuse_devices_changed` 同源）。 */
  version: number;
  devices: EchomuseDevice[];
}

/**
 * 拉取 EchoMuse 设备清单（G1）。判形状 `{enabled:boolean, devices[]}`，
 * **不判 `ok===true`（响应无 ok 键）也不判 HTTP 200**。
 */
export async function getEchomuseDevices(): Promise<EchomuseDevices> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("echomuse/devices");
  } catch (error) {
    gateError(error, "读取 EchoMuse 设备列表");
  }
  if (!isRecord(raw) || typeof raw["enabled"] !== "boolean" || !Array.isArray(raw["devices"])) {
    throw new Error("读取 EchoMuse 设备列表：响应形状不符（缺少 enabled/devices 数组）。");
  }
  const devices: EchomuseDevice[] = [];
  for (const item of raw["devices"]) {
    if (isRecord(item)) {
      devices.push(parseDevice(item));
    }
  }
  return {
    enabled: raw["enabled"],
    version: num(raw, "version"),
    devices,
  };
}

// ============================================================
// D.1 #6 GET /em/status（G0 无鉴权特例）
// ============================================================

/** `/em/status` 归一后的响应（enabled=false 时仍是 200，非 404）。 */
export interface EmStatus {
  enabled: boolean;
  /** hub 当前 key = 处于会话中的在线设备。 */
  devices: { deviceId: string; connectedSecs: number }[];
}

/**
 * 读取 `/em/status`（G0 **无鉴权 handler**：头注释显式声明——不写 401/token 分支，
 * 不注入任何凭据）。注意：该端点**不在 `/api` 前缀下**，不能用 requestApi
 * （它会拼 `/api/...`），走相对路径 fetch（vite proxy `/em` 与生产同源均可达）。
 * 编入时 enabled=false 仍 200 JSON；未编入时落 HTML 200 → SyntaxError 归一。
 */
export async function getEmStatus(): Promise<EmStatus> {
  let raw: unknown;
  try {
    const response = await fetch("/em/status", { headers: { Accept: "application/json" } });
    const text = await response.text();
    if (!response.ok) {
      throw new Error(`读取 /em/status 失败（HTTP ${response.status}）`);
    }
    raw = JSON.parse(text) as unknown;
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new EchomuseUnavailableError(
        "读取 /em/status：响应不是 JSON（通常构建未编入 echomuse-plugin，请求被管理页 HTML 回退接走）。",
        "not-compiled",
      );
    }
    throw error instanceof Error ? error : new Error(String(error));
  }
  if (!isRecord(raw) || typeof raw["enabled"] !== "boolean" || !Array.isArray(raw["devices"])) {
    throw new Error("读取 /em/status：响应形状不符（缺少 enabled/devices 数组）。");
  }
  const devices: { deviceId: string; connectedSecs: number }[] = [];
  for (const item of raw["devices"]) {
    if (isRecord(item)) {
      devices.push({ deviceId: str(item, "device_id"), connectedSecs: num(item, "connected_secs") });
    }
  }
  return { enabled: raw["enabled"], devices };
}

// ============================================================
// B.2/B.3 单设备读取与删除（G1）
// ============================================================

/** 读取单台设备（`record_payload`；404 `device not found` → EchomuseApiError）。 */
export async function getEchomuseDevice(deviceId: string): Promise<EchomuseDevice> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`echomuse/devices/${encodeURIComponent(deviceId)}`);
  } catch (error) {
    gateError(error, "读取 EchoMuse 设备详情");
  }
  if (!isRecord(raw) || typeof raw["device_id"] !== "string") {
    throw new Error("读取 EchoMuse 设备详情：响应形状不符（缺少 device_id）。");
  }
  return parseDevice(raw);
}

/**
 * 删除设备登记记录（POST `/{id}/delete` 别名——同 OTA 惯例；服务端另有原生 DELETE，
 * 行为一致）。判 `{removed:true}`。
 */
export async function deleteEchomuseDevice(deviceId: string): Promise<void> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`echomuse/devices/${encodeURIComponent(deviceId)}/delete`, {
      method: "POST",
    });
  } catch (error) {
    gateError(error, "删除 EchoMuse 设备记录");
  }
  if (!isRecord(raw) || raw["removed"] !== true) {
    throw new Error("删除 EchoMuse 设备记录：服务端未确认删除（响应缺少 removed:true）。");
  }
}

// ============================================================
// B.4 批准/认领（G1；认领与改名同一端点）
// ============================================================

/**
 * 批准（认领）一台设备；`label` 可选（缺省发 `{}`），同一端点可用于改名。
 * 成功回 `record_payload`。
 */
export async function approveEchomuseDevice(deviceId: string, label?: string): Promise<EchomuseDevice> {
  const body =
    typeof label === "string" && label.trim() !== ""
      ? { label: label.trim() }
      : {};
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`echomuse/devices/${encodeURIComponent(deviceId)}/approve`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch (error) {
    gateError(error, "批准 EchoMuse 设备");
  }
  if (!isRecord(raw) || typeof raw["device_id"] !== "string") {
    throw new Error("批准 EchoMuse 设备：响应形状不符（缺少 device_id）。");
  }
  return parseDevice(raw);
}

// ============================================================
// B.5 指令下发（G1；action 白名单 wifi/wifi_scan/wifi_commit，reboot 拒绝）
// ============================================================

/** 指令下发结果（202 `{"queued":true}`——排队 ≠ 应用）。 */
export interface EchomuseCommandResult {
  queued: boolean;
}

/**
 * 下发 WiFi 指令（`{"action":"wifi"|"wifi_scan"|"wifi_commit","ssid"?,"psk"?}`）。
 * `action=reboot` 服务端明确拒绝（要求走 system/reboot），本方法不拦截、原样提交。
 * 在线 202 `{queued:true}`；离线 409 `{"error":"device offline"}` → EchomuseApiError。
 */
export async function sendEchomuseCommand(
  deviceId: string,
  body: { action: "wifi" | "wifi_scan" | "wifi_commit"; ssid?: string; psk?: string },
): Promise<EchomuseCommandResult> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`echomuse/devices/${encodeURIComponent(deviceId)}/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch (error) {
    gateError(error, "下发 EchoMuse 指令");
  }
  if (!isRecord(raw) || raw["queued"] !== true) {
    throw new Error("下发 EchoMuse 指令：服务端未确认排队（响应缺少 queued:true）。");
  }
  return { queued: true };
}

// ============================================================
// B.6 settings（G1；40 键白名单；UI 本波不暴露编辑——/em/ 官方页承载）
// ============================================================

/**
 * 保存设备设置（部分更新）。成功回 `{"persisted":true,"delivered":bool}`：
 * 在线 202 / 离线 200 都持久化；`delivered` 是逐 ControlOut 发送结果（不是 online 布尔）。
 * **UI 本波不暴露编辑表单**（配置编辑由 `/em/` 官方页 config 页签承载），方法按 wire 补齐。
 */
export async function saveEchomuseSettings(
  deviceId: string,
  body: Record<string, JsonValue>,
): Promise<{ persisted: boolean; delivered: boolean }> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`echomuse/devices/${encodeURIComponent(deviceId)}/settings`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch (error) {
    gateError(error, "保存 EchoMuse 设备设置");
  }
  if (!isRecord(raw) || raw["persisted"] !== true || typeof raw["delivered"] !== "boolean") {
    throw new Error("保存 EchoMuse 设备设置：响应形状不符（缺少 persisted/delivered）。");
  }
  return { persisted: true, delivered: raw["delivered"] };
}

// ============================================================
// system 5 端点（B.8–B.12）
// ============================================================

/** runtime 归一视图（诊断页签展示 + 复制原文用）。 */
export interface EchomuseRuntime {
  /** 服务端回包原样（复制诊断内容用它，前端不编造字段）。 */
  raw: JsonValue;
  online: boolean;
  sessionId: string | null;
  updatedAt: string | null;
  /** `link{authenticated,token_required}`（离线为 null；无设备侧 TLS 位）。 */
  link: { authenticated: boolean; tokenRequired: boolean } | null;
}

/**
 * 读取设备运行时（B.8：**G1 + registry 存在性，不是 system_gate**）。
 * 离线回 `offline_payload`（全 null，显式不缺键）→ online=false。
 */
export async function getEchomuseRuntime(deviceId: string): Promise<EchomuseRuntime> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`echomuse/devices/${encodeURIComponent(deviceId)}/runtime`);
  } catch (error) {
    gateError(error, "读取 EchoMuse 设备报告");
  }
  if (!isRecord(raw)) {
    throw new Error("读取 EchoMuse 设备报告：响应形状不符（不是 JSON 对象）。");
  }
  const linkRaw = isRecord(raw["link"]) ? raw["link"] : null;
  const updatedAt = raw["updated_at"];
  const sessionId = raw["session_id"];
  return {
    raw: raw as JsonValue,
    online: raw["online"] === true,
    sessionId: typeof sessionId === "string" ? sessionId : null,
    updatedAt: typeof updatedAt === "string" ? updatedAt : null,
    link: linkRaw
      ? {
          authenticated: linkRaw["authenticated"] === true,
          tokenRequired: linkRaw["token_required"] === true,
        }
      : null,
  };
}

/**
 * 设置音量（B.9：`{"level":0..127}`，202 `{"queued":true,"applied":false,"level"}`）。
 * **UI 本波不暴露**（音量由 `/em/` 官方页承载），方法按 wire 补齐。
 */
export async function setEchomuseVolume(
  deviceId: string,
  level: number,
): Promise<{ queued: boolean; applied: boolean; level: number }> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`echomuse/devices/${encodeURIComponent(deviceId)}/volume`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ level }),
    });
  } catch (error) {
    gateError(error, "设置 EchoMuse 设备音量");
  }
  if (!isRecord(raw) || raw["queued"] !== true) {
    throw new Error("设置 EchoMuse 设备音量：服务端未确认排队（响应缺少 queued:true）。");
  }
  return { queued: true, applied: raw["applied"] === true, level: num(raw, "level") };
}

/** 系统命令输出的通用形状（B.10/B.11：`{"device_id","output","exit_code"}`）。 */
export interface EchomuseShellOutput {
  deviceId: string;
  output: string;
  exitCode: number;
}

function parseShellOutput(raw: unknown, action: string): EchomuseShellOutput {
  if (!isRecord(raw) || typeof raw["output"] !== "string") {
    throw new Error(`${action}：响应形状不符（缺少 output）。`);
  }
  return { deviceId: str(raw, "device_id"), output: raw["output"], exitCode: num(raw, "exit_code") };
}

/** 读取原始系统信息（B.10，G2 `system_gate` 五重；固定 INFO 命令）。 */
export async function getEchomuseSystemInfo(deviceId: string): Promise<EchomuseShellOutput> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`echomuse/devices/${encodeURIComponent(deviceId)}/system/info`);
  } catch (error) {
    gateError(error, "读取 EchoMuse 原始系统信息");
  }
  return parseShellOutput(raw, "读取 EchoMuse 原始系统信息");
}

/** system/logs 六源白名单（system.rs:27-37）。 */
export const ECHOMUSE_LOG_SOURCES = [
  "server",
  "messages",
  "messages_last",
  "last_kmsg_prev",
  "supervisor",
  "net",
] as const;

export type EchomuseLogSource = (typeof ECHOMUSE_LOG_SOURCES)[number];

/**
 * 读取设备日志（B.11，G2；`?source=` 白名单，缺省 messages）。
 * **UI 本波不暴露**（日志由 `/em/` 官方页 logs 页签承载），方法按 wire 补齐。
 */
export async function getEchomuseSystemLogs(
  deviceId: string,
  source: EchomuseLogSource = "messages",
): Promise<EchomuseShellOutput & { source: string }> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(
      `echomuse/devices/${encodeURIComponent(deviceId)}/system/logs?source=${encodeURIComponent(source)}`,
    );
  } catch (error) {
    gateError(error, "读取 EchoMuse 设备日志");
  }
  const parsed = parseShellOutput(raw, "读取 EchoMuse 设备日志");
  return { ...parsed, source: isRecord(raw) ? str(raw, "source") || source : source };
}

/**
 * 重启设备（B.12，G2；**精确** `{"confirm":true}`）。
 * 202 `{dispatched:true,rebooted:false}`——确认调度 ≠ 已完成，UI 不得报「已重启」。
 */
export async function rebootEchomuseDevice(
  deviceId: string,
): Promise<{ dispatched: boolean; rebooted: boolean }> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(
      `echomuse/devices/${encodeURIComponent(deviceId)}/system/reboot`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ confirm: true }),
      },
    );
  } catch (error) {
    gateError(error, "重启 EchoMuse 设备");
  }
  if (!isRecord(raw) || raw["dispatched"] !== true) {
    throw new Error("重启 EchoMuse 设备：服务端未确认调度（响应缺少 dispatched:true）。");
  }
  return { dispatched: true, rebooted: raw["rebooted"] === true };
}

// ============================================================
// oww 2 端点（B.13/B.14）
// ============================================================

/** OWW 资产单件状态（五态聚合之一件）。 */
export interface OwwAssetState {
  state: string;
  file: string;
  sha256: string | null;
  bytes: number | null;
}

/** OWW 五态聚合（B.14：读不到设备一律 unknown，绝不假填 missing/installed）。 */
export interface OwwState {
  deviceId: string;
  vad: OwwAssetState;
  ort: OwwAssetState;
  freeMb: number | null;
  /** 服务端自描述的五值全集（installed/outdated/blocked/unknown/missing）。 */
  states: string[];
  checkedAt: number;
}

function parseAsset(raw: unknown): OwwAssetState {
  if (!isRecord(raw)) {
    return { state: "unknown", file: "", sha256: null, bytes: null };
  }
  const sha256 = raw["sha256"];
  const bytes = raw["bytes"];
  return {
    state: str(raw, "state") || "unknown",
    file: str(raw, "file"),
    sha256: typeof sha256 === "string" ? sha256 : null,
    bytes: typeof bytes === "number" && Number.isFinite(bytes) ? bytes : null,
  };
}

/** 读取 OWW 资产状态（B.14，G2）。读不到设备不是错误（200 + unknown）。 */
export async function getOwwState(deviceId: string): Promise<OwwState> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`echomuse/devices/${encodeURIComponent(deviceId)}/oww/state`);
  } catch (error) {
    gateError(error, "读取 OWW 资产状态");
  }
  if (!isRecord(raw) || !isRecord(raw["vad"]) || !isRecord(raw["ort"])) {
    throw new Error("读取 OWW 资产状态：响应形状不符（缺少 vad/ort）。");
  }
  const freeMb = raw["free_mb"];
  return {
    deviceId: str(raw, "device_id"),
    vad: parseAsset(raw["vad"]),
    ort: parseAsset(raw["ort"]),
    freeMb: typeof freeMb === "number" && Number.isFinite(freeMb) ? freeMb : null,
    states: strArray(raw, "states"),
    checkedAt: num(raw, "checked_at"),
  };
}

/**
 * 安装 VAD 资产（B.13，G2；服务端自己下载 silero_vad.onnx 并经受限 shell 推流，
 * 不依赖客户端上传）。成功回 `{installed:true,...,applies:"next-turn"}`。
 * **UI 本波不暴露**（资产安装由 `/em/` 官方页 updates 页签承载；旧端注释
 * 「官方已有的资产安装不再重复设置入口」），方法按 wire 补齐。
 */
export async function installOwwVad(deviceId: string): Promise<void> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(
      `echomuse/devices/${encodeURIComponent(deviceId)}/oww/install-vad`,
      { method: "POST" },
    );
  } catch (error) {
    gateError(error, "安装 OWW VAD 资产");
  }
  if (!isRecord(raw) || raw["installed"] !== true) {
    throw new Error("安装 OWW VAD 资产：服务端未确认安装（响应缺少 installed:true）。");
  }
}
