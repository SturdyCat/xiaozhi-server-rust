/**
 * 插件域（W11）端点封装：`server/src/gateway/ws/plugins.rs`（555 行）+
 * `wasm_api/lifecycle.rs`（360 行）+ `plugins/wasm/lifecycle.rs:474-537`（snapshot_json 投影），
 * 全部 wire 形状逐行核实。
 *
 * ## 11 个端点 wire 表（3 个不做，仅登记）
 *
 * | # | 端点 | 方法 | 实现 |
 * |---|------|------|------|
 * | 1 | `/api/plugins` | GET | `getPluginsFull`（无条件基础路由，永远 JSON） |
 * | 2 | `/api/plugins/{id}/ui` | GET | **不做**（W11 以清单内 `registration.ui` 透传为源，不发第二个请求） |
 * | 3 | `/api/plugins/{id}/actions/{name}` | POST | `runPluginAction`（统一动作端点） |
 * | 4 | `/api/plugins/extensions/{point}/preference` | POST | **不做**（扩展点首选不在本轮范围） |
 * | 5 | `/api/plugins/upload` | POST(JSON base64) | **不做**（管理台走 raw 直传） |
 * | 6 | `/api/plugins/upload/raw` | POST(octet-stream) | `uploadPluginRaw`（id 缺省由 manifest 探测，不传 `?id=`） |
 * | 7 | `/api/plugins/{id}/register` | POST | `registerPlugin` |
 * | 8 | `/api/plugins/{id}/activate` | POST | `activatePlugin` |
 * | 9 | `/api/plugins/{id}/deactivate` | POST | `deactivatePlugin` |
 * | 10 | `/api/plugins/{id}/remove` | POST | `removePlugin` |
 * | 11 | `/api/plugins/{id}/unload` | POST | **不做**（兼容别名，客户端应改用 deactivate/remove） |
 *
 * ## 生命周期三态机
 *
 * `staged`（待注册）→ `registered`（已注册·停用）→ `active`（运行中）。
 * `registration`（menu/config_fields/actions/ui）仅 Active 投影，其余生命周期 wire 上是
 * `"registration": null`——解析必须用 isRecord 守卫，`?.` 链遇到 JSON null 会把整份
 * 快照炸成空表（PluginSlots.kt:41-45 陷阱）。
 *
 * ## fail-closed 双路径（检查单① W11 特化）
 *
 * - **清单（端点 1）是 JSON，不判 HTML 200**：无条件基础路由，判 `ok===true` + `ui`
 *   形状即可；`wasm` 键缺失/损坏 → null（静态基线 fail-open），**不是错误**。
 * - **写操作（端点 3/6/7/8/9/10）有 feature 门控**：未编入 `wasm-plugins` 时路由不存在，
 *   请求可能被管理页 HTML 回退接走——catch 链必须含 `SyntaxError → PluginsUnavailableError`
 *   归一（requestApi 对 HTML 200 抛裸 SyntaxError，api.ts:141），与「JSON 解析成功但
 *   形状不符」并列。
 *
 * ## 鉴权清单（检查单②，逐 handler；与 W10 结论相反）
 *
 * 11 个 handler **全部不调 auth_ok**（Kuikly 端走 WS RPC 握手鉴权；admin-web REST 直连
 * 不校验 token，见 plugins.rs 全文）。因此本文件**不写任何 401 分支**；方法头不声明
 * 401 语义。请求照常经 requestApi / XHR 注入 `Authorization: Bearer`（服务端忽略，无害），
 * token 不进 URL。
 *
 * 体积上限**不写死**（不预检文件大小）：`MAX_ARTIFACT_BYTES` 是服务端权威，超限由
 * 413 纯文本「产物超上限（N 字节 > M）」透出。
 */

import { ApiError, apiGet, apiPost, apiUrl, getAuthToken } from "@/lib/api";
import type { JsonValue } from "@/lib/configApi";

/** 插件平台结构性不可用（HTML 200 归一 / 形状不符），UI 层按能力未启用呈现。 */
export class PluginsUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PluginsUnavailableError";
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

/** 尽力从错误响应体取 `{ok:false,error}` 的 error；纯文本兜底原文。 */
function readServerError(body: string): string | null {
  const text = body.trim();
  if (text === "") {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(text);
    if (isRecord(parsed)) {
      const error = parsed["error"];
      if (typeof error === "string" && error.trim() !== "") {
        return error.trim();
      }
    }
  } catch {
    // 非 JSON 错误体：返回原文。
  }
  return text;
}

/**
 * 写操作错误归一（检查单①强制落地）：catch 链同时覆盖
 * ① `SyntaxError → PluginsUnavailableError`（HTML 200 被 requestApi 裸抛）；
 * ② `ApiError` 读服务端 `{ok:false,error}` 透出（409 的 missing/dependents/lifecycle
 *    细节都在 error 文案里，服务端已拼好人话，原样透传）；
 * ③ 404 空体补「未编入 wasm-plugins」提示。
 */
function normalizeWriteError(error: unknown, action: string): never {
  if (error instanceof SyntaxError) {
    throw new PluginsUnavailableError(
      `${action}失败：服务端返回了非 JSON 内容（通常是构建未编入 wasm-plugins，请求被管理页 HTML 回退接走）。按能力未启用处理，不伪装成操作成功。`,
    );
  }
  if (error instanceof ApiError) {
    const detail = readServerError(error.body);
    if (detail !== null) {
      throw new Error(`${action}失败：${detail}`);
    }
    if (error.status === 404) {
      throw new Error(`${action}失败：服务端无此端点（HTTP 404，可能未编入 wasm-plugins）。`);
    }
    throw new Error(`${action}失败（HTTP ${error.status}）。`);
  }
  throw error instanceof Error ? error : new Error(`${action}失败：${String(error)}`);
}

// ============================================================
// GET /api/plugins（清单投影 + wasm 快照解析）
// ============================================================

/** 一条依赖声明（`requires[]` 行）；`satisfied` 由服务端按当前注册表求值下发。 */
export interface PluginRequirement {
  id: string;
  minVersion: string;
  satisfied: boolean;
  /** 依赖插件当前激活版本；null = 未激活/未装载（fail-closed 展示口径）。 */
  activeVersion: string | null;
}

/** 注册菜单项全量（`menu[]` 行）。 */
export interface PluginMenuItem {
  slot: string;
  id: string;
  label: string;
  group: string;
  order: number;
  icon: string;
}

/** 注册动作声明（`actions[]` 行）；danger/help 元数据只取声明。 */
export interface PluginAction {
  id: string;
  label: string;
  danger: boolean;
  help: string;
}

/** 注册配置字段声明（`config_fields[]` 行）；客户端无配置读写通道，只展示元数据。 */
export interface PluginConfigField {
  key: string;
  label: string;
  kind: string;
  help: string;
  defaultHint: string;
  secret: boolean;
}

/** 一条侧栏动态导航（`menu[]` 中 `slot === "nav.main"` 的行）。 */
export interface PluginNavEntry {
  pluginId: string;
  entryId: string;
  label: string;
  group: string;
  order: number;
  icon: string;
  /** 旧端导航 id 形态（`plugin:{pluginId}:{entryId}`）；URL 路径由调用方派生。 */
  navigationId: string;
}

/** 扩展点解析状态（`wasm.extensions` 一行）。 */
export interface PluginExtensionState {
  state: string;
  implementation: string;
  preferred: string;
}

/** Active 插件的注册投影（`registration` 仅 Active 非 null）。 */
export interface PluginRegistration {
  menu: PluginMenuItem[];
  configFields: PluginConfigField[];
  actions: PluginAction[];
  /** A2UI 文档对象原文（`registration.ui` 透传；解析见 pluginUi.ts）。 */
  ui: JsonValue | null;
}

/** `wasm.plugins[]` 一行（snapshot_json 投影 19 字段全量）。 */
export interface PluginRecord {
  id: string;
  version: string;
  /** 宿主契约版本（如 "0.1"）；旧服务端可能不下发，空串 = 未知。 */
  apiVersion: string;
  title: string;
  description: string;
  official: boolean;
  implements: string[];
  permissions: string[];
  subscriptions: string[];
  endpoints: string[];
  resources: string[];
  configNamespace: string;
  provides: string[];
  services: string[];
  /** staged | registered | active；null = 旧服务端未投影（fail-open：语义等价 active）。 */
  lifecycle: string | null;
  /** phase 仅在 active 生命周期有意义；非 Active 投影 `{"state":"inactive"}`。 */
  phase: { state: string };
  registration: PluginRegistration | null;
  requires: PluginRequirement[];
  /** 激活中依赖本插件的其他插件 id（停用前置条件）。 */
  dependents: string[];

  // —— 派生（对齐 PluginSlots.kt PluginSlotPlugin）——
  isActive: boolean;
  canActivate: boolean;
  canDeactivate: boolean;
  canRemove: boolean;
  missingRequirements: PluginRequirement[];
  lifecycleLabel: string;
}

/** `GET /api/plugins` 回包的 `wasm` 键解析结果；null = 静态基线（fail-open）。 */
export interface WasmSnapshot {
  plugins: PluginRecord[];
  extensions: Record<string, PluginExtensionState>;
  navEntries: PluginNavEntry[];
}

/** `GET /api/plugins` 归一后的响应。 */
export interface PluginsFullSnapshot {
  capabilities: { id: string; enabled: boolean }[];
  /** "pure-base"（纯底座）| "base"（携带静态业务）。 */
  uiMode: string;
  staticBusiness: boolean;
  configPath: string | null;
  configPersistent: boolean | null;
  /** wasm 键缺失/损坏 → null：静态基线，不是错误。 */
  wasm: WasmSnapshot | null;
}

/** 安全读取字符串数组。 */
function strArray(raw: Record<string, unknown>, key: string): string[] {
  const value = raw[key];
  if (!Array.isArray(value)) {
    return [];
  }
  return value.filter((item): item is string => typeof item === "string" && item !== "");
}

function parseMenu(raw: unknown): PluginMenuItem[] {
  if (!Array.isArray(raw)) {
    return [];
  }
  const out: PluginMenuItem[] = [];
  for (const item of raw) {
    if (!isRecord(item)) {
      continue;
    }
    const id = str(item, "id");
    if (id === "") {
      continue;
    }
    const orderRaw = item["order"];
    out.push({
      slot: str(item, "slot"),
      id,
      label: str(item, "label") || id,
      group: str(item, "group"),
      order: typeof orderRaw === "number" && Number.isFinite(orderRaw) ? orderRaw : 0,
      icon: str(item, "icon") || "settings",
    });
  }
  return out;
}

function parseActions(raw: unknown): PluginAction[] {
  if (!Array.isArray(raw)) {
    return [];
  }
  const out: PluginAction[] = [];
  for (const item of raw) {
    if (!isRecord(item)) {
      continue;
    }
    const id = str(item, "id");
    if (id === "") {
      continue;
    }
    out.push({
      id,
      label: str(item, "label") || id,
      danger: item["danger"] === true,
      help: str(item, "help"),
    });
  }
  return out;
}

function parseConfigFields(raw: unknown): PluginConfigField[] {
  if (!Array.isArray(raw)) {
    return [];
  }
  const out: PluginConfigField[] = [];
  for (const item of raw) {
    if (!isRecord(item)) {
      continue;
    }
    const key = str(item, "key");
    if (key === "") {
      continue;
    }
    out.push({
      key,
      label: str(item, "label") || key,
      kind: str(item, "kind") || "str",
      help: str(item, "help"),
      defaultHint: str(item, "default_hint"),
      secret: item["secret"] === true,
    });
  }
  return out;
}

function parseRequires(raw: unknown): PluginRequirement[] {
  if (!Array.isArray(raw)) {
    return [];
  }
  const out: PluginRequirement[] = [];
  for (const item of raw) {
    if (!isRecord(item)) {
      continue;
    }
    const id = str(item, "id");
    if (id === "") {
      continue;
    }
    const activeVersion = item["active_version"];
    out.push({
      id,
      minVersion: str(item, "min_version"),
      satisfied: item["satisfied"] === true,
      // JSON null / 缺键 → null（未激活/未装载）。
      activeVersion: typeof activeVersion === "string" ? activeVersion : null,
    });
  }
  return out;
}

/** 解析 `wasm` 键；`plugins` 数组缺失/损坏 → null（静态基线 fail-open）。 */
function parseWasmSnapshot(wasm: Record<string, unknown>): WasmSnapshot | null {
  const rows = wasm["plugins"];
  if (!Array.isArray(rows)) {
    return null;
  }
  const plugins: PluginRecord[] = [];
  const navEntries: PluginNavEntry[] = [];
  for (const row of rows) {
    if (!isRecord(row)) {
      continue;
    }
    const id = str(row, "id");
    if (id === "") {
      continue;
    }
    // JsonNull 陷阱：非 Active 投影 `"registration": null` —— 一律 isRecord 守卫。
    const registrationRaw = isRecord(row["registration"]) ? row["registration"] : null;
    const registration: PluginRegistration | null = registrationRaw
      ? {
          menu: parseMenu(registrationRaw["menu"]),
          configFields: parseConfigFields(registrationRaw["config_fields"]),
          actions: parseActions(registrationRaw["actions"]),
          ui: (registrationRaw["ui"] ?? null) as JsonValue,
        }
      : null;
    const phaseRaw = row["phase"];
    const phase = {
      state: isRecord(phaseRaw) ? str(phaseRaw, "state") || "unknown" : "unknown",
    };
    const requires = parseRequires(row["requires"]);
    const dependents = strArray(row, "dependents");
    const lifecycleRaw = row["lifecycle"];
    const lifecycle = typeof lifecycleRaw === "string" && lifecycleRaw !== "" ? lifecycleRaw : null;
    const missingRequirements = requires.filter((item) => !item.satisfied);
    const plugin: PluginRecord = {
      id,
      version: str(row, "version"),
      apiVersion: str(row, "api_version"),
      title: str(row, "title") || id,
      description: str(row, "description"),
      official: row["official"] === true,
      implements: strArray(row, "implements"),
      permissions: strArray(row, "permissions"),
      subscriptions: strArray(row, "subscriptions"),
      endpoints: strArray(row, "endpoints"),
      resources: strArray(row, "resources"),
      configNamespace: str(row, "config_namespace"),
      provides: strArray(row, "provides"),
      services: strArray(row, "services"),
      lifecycle,
      phase,
      registration,
      requires,
      dependents,
      // 派生口径与 PluginSlots.kt:206-226 逐条对齐。
      isActive: lifecycle === null || lifecycle === "active",
      canActivate: lifecycle === "registered" && missingRequirements.length === 0,
      canDeactivate: lifecycle === "active" && dependents.length === 0,
      canRemove: lifecycle !== null && lifecycle !== "active",
      missingRequirements,
      lifecycleLabel:
        lifecycle === "staged"
          ? "待注册"
          : lifecycle === "registered"
            ? "已注册·已停用"
            : lifecycle === "active" || lifecycle === null
              ? "运行中"
              : lifecycle,
    };
    plugins.push(plugin);
    if (registration !== null) {
      for (const menu of registration.menu) {
        if (menu.slot !== "nav.main") {
          continue;
        }
        navEntries.push({
          pluginId: id,
          entryId: menu.id,
          label: menu.label,
          group: menu.group || "插件",
          order: menu.order,
          icon: menu.icon,
          navigationId: `plugin:${id}:${menu.id}`,
        });
      }
    }
  }
  const extensions: Record<string, PluginExtensionState> = {};
  const extensionsRaw = wasm["extensions"];
  if (isRecord(extensionsRaw)) {
    for (const [key, value] of Object.entries(extensionsRaw)) {
      if (!isRecord(value)) {
        continue;
      }
      extensions[key] = {
        state: str(value, "state") || "unresolved",
        implementation: str(value, "implementation"),
        preferred: str(value, "preferred"),
      };
    }
  }
  return { plugins, extensions, navEntries };
}

/**
 * 拉取插件清单（`GET /api/plugins`）。**清单是无条件基础路由，永远 JSON**：
 * 判 `ok===true` + `ui` 形状，不判 HTTP 200、不写 HTML 归一分支（检查单 W11 特化）。
 * `wasm` 键缺失/损坏 → wasm=null（静态基线），不是错误。
 */
export async function getPluginsFull(): Promise<PluginsFullSnapshot> {
  const raw = await apiGet<unknown>("plugins");
  if (!isRecord(raw) || raw["ok"] !== true) {
    throw new Error("插件清单加载失败：响应缺少 ok:true，不视为有效清单。");
  }
  const ui = raw["ui"];
  if (!isRecord(ui)) {
    throw new Error("插件清单加载失败：响应缺少 ui 投影（服务端版本过旧）。");
  }
  const capabilities: { id: string; enabled: boolean }[] = [];
  const capsRaw = raw["capabilities"];
  if (Array.isArray(capsRaw)) {
    for (const item of capsRaw) {
      if (!isRecord(item)) {
        continue;
      }
      const id = str(item, "id");
      if (id === "") {
        continue;
      }
      capabilities.push({ id, enabled: item["enabled"] === true });
    }
  }
  const configRaw = raw["config"];
  const configPath =
    isRecord(configRaw) && typeof configRaw["path"] === "string"
      ? (configRaw["path"] as string)
      : null;
  return {
    capabilities,
    uiMode: str(ui, "mode") || "unknown",
    staticBusiness: ui["static_business"] === true,
    configPath,
    configPersistent: isRecord(configRaw) ? configRaw["persistent"] === true : null,
    wasm: isRecord(raw["wasm"]) ? parseWasmSnapshot(raw["wasm"] as Record<string, unknown>) : null,
  };
}

// ============================================================
// POST /api/plugins/upload/raw（raw octet-stream 直传）
// ============================================================

/** 上传成功回包（`adopt_bytes` 成功分支 :437-453，字段一一对应）。 */
export interface PluginUploadResult {
  id: string;
  staged: boolean;
  /** 采用后的生命周期（恒 staged；原 Active 条目升级后仍 active）。 */
  lifecycle: string;
  version: string;
  /** 同 id 升级（新 > 旧才接受）。 */
  upgraded: boolean;
  /** 升级前的版本号；非升级为 null。 */
  previousVersion: string | null;
  sha256: string;
  /** 落盘 + 摘要钉版都成功；false 时本次运行有效、重启可能丢失。 */
  persisted: boolean;
  /** 原 Active 条目原地换版本失败原因；成功为 null。 */
  reloadError: string | null;
}

/**
 * 上传 `.wasm` 组件（裸字节 `application/octet-stream`）。**不传 `?id=`**：id 缺省由
 * 服务端从组件 manifest 探测（「选文件即上传」）。onProgress 回真实百分比（XHR
 * `upload.onprogress`）。**体积不写死**：不预检大小，服务端 413 纯文本「产物超上限」
 * 原样透出。403 是平台总闸 `{ok:false,error}`、409 是版本不升级 `{ok:false,error,...}`，
 * 都读 error 字段透出。无鉴权 handler（检查单②）：不写 401 分支，Bearer 照常注入。
 */
export function uploadPluginRaw(
  file: File,
  onProgress?: (percent: number) => void,
): Promise<PluginUploadResult> {
  return new Promise<PluginUploadResult>((resolve, reject) => {
    const url = apiUrl("plugins/upload/raw");
    const xhr = new XMLHttpRequest();
    xhr.open("POST", url);
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    const token = getAuthToken();
    if (token !== null) {
      xhr.setRequestHeader("Authorization", `Bearer ${token}`);
    }
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable && event.total > 0) {
        onProgress?.(Math.round((event.loaded / event.total) * 100));
      }
    };
    xhr.onerror = () => reject(new ApiError("上传失败（网络错误）", 0, ""));
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        let parsed: unknown;
        try {
          parsed = JSON.parse(xhr.responseText);
        } catch {
          reject(
            new ApiError("上传失败：服务端返回了无法解析的响应", xhr.status, xhr.responseText),
          );
          return;
        }
        if (!isRecord(parsed) || parsed["ok"] !== true || parsed["staged"] !== true) {
          reject(
            new ApiError(
              "上传失败：服务端未确认登记 Staged（响应缺少 ok/staged:true）。",
              xhr.status,
              xhr.responseText,
            ),
          );
          return;
        }
        const previousVersion = parsed["previous_version"];
        const reloadError = parsed["reload_error"];
        resolve({
          id: str(parsed, "id"),
          staged: true,
          lifecycle: str(parsed, "lifecycle") || "staged",
          version: str(parsed, "version"),
          upgraded: parsed["upgraded"] === true,
          previousVersion: typeof previousVersion === "string" ? previousVersion : null,
          sha256: str(parsed, "sha256"),
          persisted: parsed["persisted"] === true,
          reloadError: typeof reloadError === "string" ? reloadError : null,
        });
        return;
      }
      // 413 纯文本「产物超上限」/ 403、409、400 JSON `{ok:false,error}`：读 error 字段，纯文本兜底。
      reject(
        new ApiError(
          `上传失败：${readServerError(xhr.responseText) ?? `HTTP ${xhr.status}`}`,
          xhr.status,
          xhr.responseText,
        ),
      );
    };
    xhr.send(file);
  });
}

// ============================================================
// 生命周期端点（register / activate / deactivate / remove）
// ============================================================

/** 生命周期操作结果（成功响应 `{ok:true,id,lifecycle,...}`；未消费的扩展键忽略）。 */
export interface PluginLifecycleResult {
  id: string;
  lifecycle: string;
}

async function lifecycleRequest(
  pluginId: string,
  op: "register" | "activate" | "deactivate" | "remove",
  action: string,
): Promise<PluginLifecycleResult> {
  const raw = await apiPost<unknown>(`plugins/${encodeURIComponent(pluginId)}/${op}`, undefined, {
    action,
    onError: normalizeWriteError,
  });
  if (!isRecord(raw) || raw["ok"] !== true) {
    const detail = isRecord(raw) ? readServerError(JSON.stringify(raw)) : null;
    throw new Error(`${action}失败：${detail ?? "服务端未确认操作成功（响应缺少 ok:true）。"}`);
  }
  return { id: str(raw, "id") || pluginId, lifecycle: str(raw, "lifecycle") };
}

/** Staged → Registered（完整校验 + self-test；失败保留 Staged 带原因 400）。无鉴权 handler。 */
export function registerPlugin(pluginId: string): Promise<PluginLifecycleResult> {
  return lifecycleRequest(pluginId, "register", "注册");
}

/** Registered → Active（依赖前置 409 列缺口 / WrongLifecycle 409 / Build 400）。无鉴权 handler。 */
export function activatePlugin(pluginId: string): Promise<PluginLifecycleResult> {
  return lifecycleRequest(pluginId, "activate", "启用");
}

/** Active → Registered（Active 依赖者 409 列清单；产物/标记/kv 保留）。无鉴权 handler。 */
export function deactivatePlugin(pluginId: string): Promise<PluginLifecycleResult> {
  return lifecycleRequest(pluginId, "deactivate", "停用");
}

/** 仅 Staged/Registered 可移除（Active 一律 409「插件仍在运行」）。无鉴权 handler。 */
export function removePlugin(pluginId: string): Promise<PluginLifecycleResult> {
  return lifecycleRequest(pluginId, "remove", "移除");
}

// ============================================================
// POST /api/plugins/{id}/actions/{name}（统一动作端点）
// ============================================================

/** 动作执行结果（成功响应 `{ok:true,result,raw}`）。 */
export interface PluginActionRunResult {
  /** 服务端解析后的动作回包（解析失败时是字符串）。 */
  result: JsonValue;
  /** 服务端透传的动作原文（插件返回的字符串原样）。 */
  raw: string;
}

/**
 * 执行一个注册动作（payload 原样传给插件；页面无 input 时传空对象——与旧端
 * 「无输入页保持空对象」现状一致）。判 `ok===true`；400 `{ok:false,error}` 读 error
 * 透出；HTML 200 → SyntaxError → PluginsUnavailableError 归一（检查单①）。无鉴权 handler。
 */
export async function runPluginAction(
  pluginId: string,
  actionId: string,
  payload: Record<string, string>,
): Promise<PluginActionRunResult> {
  const raw = await apiPost<unknown>(
    `plugins/${encodeURIComponent(pluginId)}/actions/${encodeURIComponent(actionId)}`,
    payload,
    { action: "动作执行", onError: normalizeWriteError },
  );
  if (!isRecord(raw) || raw["ok"] !== true) {
    const detail = isRecord(raw) ? readServerError(JSON.stringify(raw)) : null;
    throw new Error(`动作执行失败：${detail ?? "服务端未确认动作成功（响应缺少 ok:true）。"}`);
  }
  return {
    result: (raw["result"] ?? null) as JsonValue,
    raw: str(raw, "raw"),
  };
}

// ============================================================
// 导航派生（AppShell 动态侧栏用）
// ============================================================

/** 把 `plugin:{pluginId}:{entryId}` 拆出插件 id；非该形态返回 null。 */
export function pluginIdFromNavigation(navigationId: string): string | null {
  if (!navigationId.startsWith("plugin:")) {
    return null;
  }
  const rest = navigationId.slice("plugin:".length);
  const pluginId = rest.slice(0, rest.indexOf(":"));
  return pluginId !== "" ? pluginId : null;
}

/** 动态导航项的 URL 路径（路由 `plugin/:pluginId/:entryId`）。 */
export function pluginNavPath(entry: PluginNavEntry): string {
  return `plugin/${encodeURIComponent(entry.pluginId)}/${encodeURIComponent(entry.entryId)}`;
}
