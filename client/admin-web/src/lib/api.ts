/**
 * 统一 API 地址入口。
 *
 * - 默认是同源相对路径 `/api`：生产由 Rust server 的 ServeDir 同源托管，
 *   浏览器直接访问相对路径，因此不存在 CORS 问题；Tauri「加载服务器页面」
 *   模式（XIAOZHI_ADMIN_URL）下窗口本身就是服务器页面，/api、/em/ 天然同源。
 * - 开发期由 Vite proxy 把 /api、/ws、/em/ 转发到 VITE_PROXY_TARGET
 *   （默认 http://127.0.0.1:8000），同样避免 CORS。
 * - 仅在需要独立联调时用 VITE_API_BASE_URL 覆盖为绝对地址。
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

/** 发起一次同源（或覆盖地址）API 请求并解析 JSON。 */
export async function requestApi<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(apiUrl(path), {
    ...init,
    headers: {
      Accept: "application/json",
      ...(init?.headers ?? {}),
    },
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
 * 推导 WebSocket 地址：同源模式下固定使用 /ws；绝对 baseURL 模式下
 * 沿用其 host 并把 http/https 换成 ws/wss。
 */
export function getWebSocketUrl(path: string): string {
  const suffix = normalizePath(path);
  const base = getApiBaseUrl();

  if (base.startsWith("http://") || base.startsWith("https://")) {
    const url = new URL(base);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.pathname = `/ws${suffix}`;
    url.search = "";
    url.hash = "";
    return url.toString();
  }

  if (typeof window === "undefined") {
    return `/ws${suffix}`;
  }
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${window.location.host}/ws${suffix}`;
}
