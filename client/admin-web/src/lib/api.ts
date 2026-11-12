/**
 * 统一 API 地址入口（默认同源相对路径 `/api`，请求实现不使用插件 IPC）。
 *
 * 三种运行模式下 baseURL 的语义：
 * - **dev**（`pnpm dev` / `pnpm tauri dev`）：窗口加载 Vite dev server
 *   `http://127.0.0.1:5173`，Vite proxy 把 `/api`、`/api/admin/ws`（管理台实时 WS）、`/em/` 转发到
 *   `VITE_PROXY_TARGET`（默认 `http://127.0.0.1:8000`），相对路径同源，API 可达。
 * - **生产默认：加载服务器页面**（方案 B，推荐）：窗口加载
 *   `XIAOZHI_ADMIN_URL`（未设置则 `http://127.0.0.1:8000`），页面本身由 Rust
 *   server 同源托管，因此相对 `/api`、`/api/admin/ws`（管理台实时 WS）、`/em/` 全部同源可达，无 CORS 问题。
 * - **`XIAOZHI_ADMIN_EMBEDDED=1` 内嵌 SPA 预览**：窗口从 `tauri://localhost`
 *   加载内嵌资产，相对 `/api` 只会打到资产协议而到不了 Rust server，
 *   **API 不可达**；该模式仅用于离线 UI 预览，不作为默认运行方式。
 *
 * 仅在需要独立联调时用 `VITE_API_BASE_URL` 覆盖为绝对地址（`getWebSocketUrl()`
 * 会据此把 http/https 换成 ws/wss）；内嵌预览模式也可借它临时指到服务器，
 * 但这属于调试手段，不改变上面的推荐运行方式。
 *
 * 管理鉴权：URL 上的 `?token=` 只作为输入来源，读取后写入 sessionStorage，
 * 并以 `Authorization: Bearer` 请求头注入（不拼进 query，避免出现在日志/历史）。
 * 该令牌只注入 `requestApi`；WebSocket / XHR 上传 / 下载链接的透传是后续收尾项。
 */

import ky, { isHTTPError } from "ky";

const DEFAULT_API_BASE_URL = "/api";

/** 去掉结尾斜杠，保证拼接结果唯一。 */
function trimTrailingSlash(value: string): string {
  return value.endsWith("/") ? trimTrailingSlash(value.slice(0, -1)) : value;
}

/** 把 "devices" 或 "/devices" 统一成 "/devices"。 */
function normalizePath(path: string): string {
  const trimmed = path.trim();
  if (trimmed === "" || trimmed === "/") {
    return "";
  }
  return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
}

/** 返回当前生效的 API baseURL。 */
export function getApiBaseUrl(): string {
  const override = import.meta.env.VITE_API_BASE_URL;
  if (typeof override === "string" && override.trim() !== "") {
    return trimTrailingSlash(override.trim());
  }
  return DEFAULT_API_BASE_URL;
}

/** 拼接一个 API 请求地址，例如 apiUrl("devices") -> "/api/devices"。 */
export function apiUrl(path: string): string {
  return `${getApiBaseUrl()}${normalizePath(path)}`;
}

/** 非 2xx 响应统一抛出的错误。 */
export class ApiError extends Error {
  public readonly status: number;
  public readonly body: string;

  constructor(message: string, status: number, body: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.body = body;
  }
}

/** sessionStorage 中复用令牌的键；只存令牌本身，不存任何凭据明文。 */
const AUTH_TOKEN_STORAGE_KEY = "admin.authToken";

/**
 * 读取管理鉴权令牌（供 `requestApi` 及后续 WS / 下载链路复用）。
 *
 * 优先级：URL 查询参数 `?token=` → 非空则写入 sessionStorage 供页面内导航后复用；
 * 未携带或为空串则回读 sessionStorage。空串一律视为 null，绝不注入空 Bearer 头。
 * 令牌**只**作为输入来源从 query 读取，不会再拼回任何请求 URL（避免进入日志/历史）。
 * 存储读写在隐私模式下可能抛异常，这里同步兜底，不让鉴权探测影响请求本身。
 */
export function getAuthToken(): string | null {
  if (typeof window === "undefined") {
    return null;
  }

  let fromQuery: string | null = null;
  try {
    fromQuery = new URLSearchParams(window.location.search).get("token");
  } catch {
    fromQuery = null;
  }
  if (fromQuery !== null && fromQuery.trim() !== "") {
    const token = fromQuery.trim();
    try {
      window.sessionStorage.setItem(AUTH_TOKEN_STORAGE_KEY, token);
    } catch {
      // sessionStorage 不可用时仅本次页面内有效，不影响请求发出。
    }
    return token;
  }

  let stored: string | null = null;
  try {
    stored = window.sessionStorage.getItem(AUTH_TOKEN_STORAGE_KEY);
  } catch {
    stored = null;
  }
  if (stored === null || stored.trim() === "") {
    return null;
  }
  return stored.trim();
}

/** 发起一次同源（或覆盖地址）API 请求并解析 JSON。 */
export async function requestApi<T>(path: string, init?: RequestInit): Promise<T> {
  // 用 Headers 保留调用方显式设置的任何头（例如 Content-Type / 自定义 Authorization），
  // 只在令牌非空且调用方未显式给出 Authorization 时注入 Bearer。
  const headers = new Headers(init?.headers);
  if (!headers.has("Accept")) {
    headers.set("Accept", "application/json");
  }
  const token = getAuthToken();
  if (token !== null && !headers.has("Authorization")) {
    headers.set("Authorization", `Bearer ${token}`);
  }

  const url = apiUrl(path);
  const response = await ky(url, {
    ...init,
    headers,
    // 对齐原 fetch 语义：一次请求绝不重试（ky 默认重试 2 次）、无超时（ky 默认每次 10s）。
    retry: 0,
    timeout: false,
    hooks: {
      beforeError: [
        async ({ error }) => {
          if (isHTTPError(error)) {
            // ky 2 在抛 HTTPError 前已把 body 全量读进 error.data（response body 被消费，
            // error.response.text() 会抛 Body is unusable）。data 的类型：JSON
            // content-type 时为已 parse 的值、其他 content-type 为原文 string、
            // 空 body/读取/解析失败为 undefined。ApiError.body 无消费方，用 stringify
            // 尽量贴近旧版「原始响应文本」语义。
            const body =
              typeof error.data === "string"
                ? error.data
                : error.data === undefined
                  ? ""
                  : JSON.stringify(error.data);
            // 注意：request.url 会被解析成绝对地址，这里保持与旧版一致用相对拼接结果。
            return new ApiError(
              `请求 ${url} 失败：HTTP ${error.response.status}`,
              error.response.status,
              body,
            );
          }
          return error;
        },
      ],
    },
  });

  const text = await response.text();
  if (text.trim() === "") {
    return undefined as T;
  }
  return JSON.parse(text) as T;
}

/** 方法级封装的错误处理选项：封装层自动捕获并归一异常，业务可覆盖。 */
export interface ApiCallOptions {
  /** 业务动作标签（如「读取设备列表」），拼入错误文案。 */
  action?: string;
  /** 业务覆盖默认归一；必须抛出（never），签名与各领域归一器一致。 */
  onError?: (error: unknown, action: string) => never;
}

/**
 * 方法级封装的默认错误归一（供未来没有领域归一器的新代码使用）。
 *
 * - `ApiError`：尽力从响应体提取服务端可读文案（`message` / `error` 字段，轻量 JSON
 *   提取、非 JSON 回落原文），拼成「<action>失败：<detail>」；无 body 时回落 HTTP 状态。
 * - `SyntaxError`（HTML 200 被 SPA fallback 接走）：明确「非 JSON 内容」。
 * - 其余异常原样重抛（不吞、不伪装成功）。
 */
function defaultNormalizeError(error: unknown, action: string): never {
  if (error instanceof ApiError) {
    let detail: string | null = null;
    const text = error.body.trim();
    if (text !== "") {
      try {
        const parsed: unknown = JSON.parse(text);
        if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) {
          const record = parsed as Record<string, unknown>;
          const message = record["message"];
          const serverError = record["error"];
          if (typeof message === "string" && message.trim() !== "") {
            detail = message.trim();
          } else if (typeof serverError === "string" && serverError.trim() !== "") {
            detail = serverError.trim();
          }
        }
      } catch {
        // 非 JSON 错误体：回落原文。
      }
      detail ??= text;
    }
    throw new Error(`${action}失败：${detail ?? `HTTP ${error.status}`}`);
  }
  if (error instanceof SyntaxError) {
    throw new Error(`${action}失败：服务端返回了非 JSON 内容（通常被管理页 HTML 回退接走）。`);
  }
  throw error instanceof Error ? error : new Error(String(error));
}

/**
 * 方法级 JSON 请求的公共实现：全部走 [`requestApi`]（鉴权注入、ApiError、
 * retry/timeout 语义都在那里），这里只收敛 method + body 的样板。
 *
 * `body === undefined` 时不发送请求体、不带 `Content-Type`（对齐既有
 * `{ method: "POST" }` 形态）；body 存在时统一 `JSON.stringify` 并带
 * `Content-Type: application/json`。
 *
 * 错误处理三层默认语义（`ApiCallOptions`）：不传 options（或 action 与 onError 均缺省）
 * 时不包 try/catch，行为与现状完全一致（异常原样上抛）；传 `onError` 时由业务归一器
 * 覆盖默认处理；只传 `action` 时由 [`defaultNormalizeError`] 归一。
 */
async function jsonRequest<T>(
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE",
  path: string,
  body?: unknown,
  options?: ApiCallOptions,
): Promise<T> {
  const promise = requestApi<T>(
    path,
    body === undefined
      ? { method }
      : {
          method,
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        },
  );
  if (options === undefined || (options.action === undefined && options.onError === undefined)) {
    return promise;
  }
  try {
    return await promise;
  } catch (error) {
    if (options.onError !== undefined) {
      // 业务覆盖：归一器签名是 never，正常实现必抛；后面 throw 兜底防实现不规范的归一器。
      options.onError(error, options.action ?? "");
      throw error;
    }
    defaultNormalizeError(error, options.action ?? "");
    throw error;
  }
}

/**
 * GET 请求（JSON 解析与鉴权同 `requestApi`）。
 *
 * 错误处理三层默认语义：传 `action` 时异常由封装层捕获并按
 * [`defaultNormalizeError`] 归一；传 `onError` 时由业务归一器覆盖默认处理；
 * 两者都缺省时行为与不带 options 完全一致（异常原样上抛）。
 */
export function apiGet<T>(path: string, options?: ApiCallOptions): Promise<T> {
  return jsonRequest<T>("GET", path, undefined, options);
}

/**
 * POST 请求；body 省略时不发送请求体，存在时统一 JSON 序列化并带 `Content-Type: application/json`。
 *
 * 错误处理三层默认语义：传 `action` 时异常由封装层捕获并按
 * [`defaultNormalizeError`] 归一；传 `onError` 时由业务归一器覆盖默认处理；
 * 两者都缺省时行为与不带 options 完全一致（异常原样上抛）。
 */
export function apiPost<T>(path: string, body?: unknown, options?: ApiCallOptions): Promise<T> {
  return jsonRequest<T>("POST", path, body, options);
}

/**
 * PUT 请求；body 省略时不发送请求体，存在时统一 JSON 序列化并带 `Content-Type: application/json`。
 *
 * 错误处理三层默认语义：传 `action` 时异常由封装层捕获并按
 * [`defaultNormalizeError`] 归一；传 `onError` 时由业务归一器覆盖默认处理；
 * 两者都缺省时行为与不带 options 完全一致（异常原样上抛）。
 */
export function apiPut<T>(path: string, body?: unknown, options?: ApiCallOptions): Promise<T> {
  return jsonRequest<T>("PUT", path, body, options);
}

/**
 * PATCH 请求；body 省略时不发送请求体，存在时统一 JSON 序列化并带 `Content-Type: application/json`。
 *
 * 错误处理三层默认语义：传 `action` 时异常由封装层捕获并按
 * [`defaultNormalizeError`] 归一；传 `onError` 时由业务归一器覆盖默认处理；
 * 两者都缺省时行为与不带 options 完全一致（异常原样上抛）。
 */
export function apiPatch<T>(path: string, body?: unknown, options?: ApiCallOptions): Promise<T> {
  return jsonRequest<T>("PATCH", path, body, options);
}

/**
 * DELETE 请求；body 省略时不发送请求体，存在时统一 JSON 序列化并带 `Content-Type: application/json`。
 *
 * 错误处理三层默认语义：传 `action` 时异常由封装层捕获并按
 * [`defaultNormalizeError`] 归一；传 `onError` 时由业务归一器覆盖默认处理；
 * 两者都缺省时行为与不带 options 完全一致（异常原样上抛）。
 */
export function apiDelete<T>(path: string, body?: unknown, options?: ApiCallOptions): Promise<T> {
  return jsonRequest<T>("DELETE", path, body, options);
}

/**
 * 推导 WebSocket 地址：同源模式下固定使用 /api/admin/ws（管理台实时通道，
 * Rust server 无裸 /ws 前缀）；绝对 baseURL 模式下沿用其 host 并把
 * http/https 换成 ws/wss，路径同样落到 /api/admin/ws。
 */
export function getWebSocketUrl(path: string): string {
  const suffix = normalizePath(path);
  const base = getApiBaseUrl();

  if (base.startsWith("http://") || base.startsWith("https://")) {
    const url = new URL(base);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.pathname = `/api/admin/ws${suffix}`;
    url.search = "";
    url.hash = "";
    return url.toString();
  }

  if (typeof window === "undefined") {
    return `/api/admin/ws${suffix}`;
  }
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${window.location.host}/api/admin/ws${suffix}`;
}
