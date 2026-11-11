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

  const response = await fetch(apiUrl(path), {
    ...init,
    headers,
  });

  const text = await response.text();
  if (!response.ok) {
    throw new ApiError(
      `请求 ${apiUrl(path)} 失败：HTTP ${response.status}`,
      response.status,
      text,
    );
  }

  if (text.trim() === "") {
    return undefined as T;
  }
  return JSON.parse(text) as T;
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
