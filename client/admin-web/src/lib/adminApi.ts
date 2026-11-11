import { ApiError, apiUrl, getAuthToken, requestApi } from "@/lib/api";

/** 服务健康信息（`GET /api/health`）。 */
export interface HealthInfo {
  status: string;
  name: string;
  version: string;
}

/** 全局运行统计（`GET /api/stats`）。 */
export interface StatsInfo {
  ok: boolean;
  generated_at_ms: number;
  uptime_ms: number;
  scope: string;
  scope_note: string;
  devices: { total: number; online: number };
  sessions: { live: number; test: number };
  conversations: { retained: number };
  turns: { retained: number; user: number; assistant: number; note: string };
  usage: unknown;
  capabilities: unknown;
  version: string;
}

/** 单个引擎能力项（`GET /api/health/engines`）。 */
export interface EngineCapability {
  capability: string;
  display: string;
  required: boolean;
  enabled: boolean;
  phase: string;
  /** wire 上是布尔（是否偏离预期实现），不是字符串。 */
  deviation: boolean;
  /** 未启用/无实现时服务端给 `null`，别当成非空字符串用。 */
  active_impl: string | null;
  last_error: string | null;
  /** 服务端权威的能力→配置分区映射；`voices` 等无对应分区时为 `null`。 */
  config_section: string | null;
  probe: unknown;
  implementations: unknown;
}

/** 引擎能力总览（`GET /api/health/engines`）。 */
export interface EnginesInfo {
  ok: boolean;
  generated_ms: number;
  counts: {
    active: number;
    degraded: number;
    failed: number;
    disabled: number;
    total: number;
  };
  capabilities: EngineCapability[];
}

/** 单个设备项（`GET /api/devices`）。 */
export interface DeviceInfo {
  device_id: string;
  client_id: string;
  online: boolean;
  last_seen_ms: number;
  board: string;
  firmware_version: string;
  is_test: boolean;
  mcp: boolean;
}

/** 设备列表（`GET /api/devices`）。 */
export interface DevicesInfo {
  ok: boolean;
  devices: DeviceInfo[];
  count: number;
  online_count: number;
  note: string;
}

/** 服务器出口地理信息（`GET /api/server/geo`）。 */
export interface GeoInfo {
  ok: boolean;
  resolved: boolean;
  ip: string;
  country: string;
  province: string;
  city: string;
  source: string;
  domestic: boolean;
  note: string;
}

/** 插件能力列表（`GET /api/plugins`）。 */
export interface PluginsInfo {
  ok: boolean;
  capabilities: { id: string; enabled: boolean }[];
  ui: { mode: string; static_business: string };
  config: { path: string; persistent: boolean };
  wasm?: unknown;
}

/** 拉取服务健康信息。 */
export const getHealth = () => requestApi<HealthInfo>("health");

/** 拉取全局运行统计。 */
export const getStats = () => requestApi<StatsInfo>("stats");

/** 拉取引擎能力总览。 */
export const getEngines = () => requestApi<EnginesInfo>("health/engines");

/** 拉取设备列表。 */
export const getDevices = () => requestApi<DevicesInfo>("devices");

/** 拉取服务器出口地理信息。 */
export const getGeo = () => requestApi<GeoInfo>("server/geo");

/** 拉取插件能力列表。 */
export const getPlugins = () => requestApi<PluginsInfo>("plugins");

/** 设备会话摘要（`GET /api/devices/{id}` 内 sessions 项）。 */
export interface DeviceSession {
  session_id: string;
  device_id: string;
  client_id: string;
  is_test: boolean;
  mcp: boolean;
  started_ms: number;
  last_activity_ms: number;
  duration_ms: number;
  idle_ms: number;
  phase: string;
  phase_display: string;
  turn_count: number;
  last_user_text: string;
  last_error: string;
  audio: unknown;
}

/** 设备对话摘要（`GET /api/devices/{id}` 内 conversations 项）。 */
export interface DeviceConversation {
  session_id: string;
  device_id: string;
  started_ms: number;
  last_ms: number;
  turn_count: number;
  truncated: boolean;
  dropped: boolean;
  turns: { role: string; text: string; at_ms: number; elapsed_ms: number }[];
}

/** 单设备详情（`GET /api/devices/{id}`），在列表项 `DeviceInfo` 基础上追加会话与对话。 */
export interface DeviceDetailInfo extends DeviceInfo {
  sessions: DeviceSession[];
  conversations: DeviceConversation[];
}

/** 消息注入结果（`POST /api/devices/{id}/inject`）。 */
export interface InjectResult {
  ok: boolean;
  device_id?: string;
  session_id?: string;
  note?: string;
  message?: string;
}

/** 拉取单个设备详情（含会话与对话）。 */
export const getDevice = (id: string) =>
  requestApi<DeviceDetailInfo>("devices/" + encodeURIComponent(id));

/** 向设备注入一条消息（≤2000 字符，由服务端校验）。 */
export const injectDevice = (id: string, text: string) =>
  requestApi<InjectResult>("devices/" + encodeURIComponent(id) + "/inject", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text }),
  });

/** 存储目录中的单个条目（`GET /api/files`）。 */
export interface FileEntry {
  name: string;
  dir: boolean;
  size: number;
  /** Unix 秒。 */
  modified: number;
  /** 受保护名（data.db 等），不可删除/覆盖。 */
  protected: boolean;
}

/** 目录列表（`GET /api/files?path=`）。 */
export interface FilesListing {
  root: string;
  path: string;
  entries: FileEntry[];
}

/** mkdir / delete 通用结果。 */
export interface FileOkResult {
  ok: boolean;
  path?: string;
  message?: string;
}

/** 上传结果（`POST /api/files/upload`）。 */
export interface UploadResult {
  ok: boolean;
  path?: string;
  size?: number;
  message?: string;
}

/** 列出目录（path 为空表示存储根）。 */
export const getFileList = (path: string) =>
  requestApi<FilesListing>(
    path ? `files?path=${encodeURIComponent(path)}` : "files",
  );

/** 新建目录（父目录一并创建，已存在幂等）。 */
export const makeDir = (path: string) =>
  requestApi<FileOkResult>("files/mkdir", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  });

/** 删除文件或目录（目录递归）。 */
export const deletePath = (path: string) =>
  requestApi<FileOkResult>("files/delete", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  });

/**
 * 上传裸字节到目标文件路径（含文件名，相对存储根），用 XHR 取得上传进度。
 * `Content-Type: application/octet-stream`，超 64MB 由服务端返回 413。
 *
 * 鉴权：`/api/files/*` 全走 `auth_ok`（`server/src/gateway/files/http.rs:80` 等
 * 五处），`server.expected_token` 非空时必须带凭据。XHR 能带自定义头，所以走
 * `Authorization: Bearer`（不进 URL/日志/历史，泄漏面最小）；`getAuthToken()`
 * 返回 null（未配置 token）时不加头，行为与注入前完全一致。
 */
export function uploadFile(
  targetPath: string,
  file: File,
  options?: {
    /** 目标已存在时是否覆盖；缺省 false，服务端返回 409。 */
    overwrite?: boolean;
    onProgress?: (loaded: number, total: number) => void;
  },
): Promise<UploadResult> {
  return new Promise<UploadResult>((resolve, reject) => {
    const overwrite = options?.overwrite === true ? "&overwrite=1" : "";
    const onProgress = options?.onProgress;
    const url = apiUrl(
      `files/upload?path=${encodeURIComponent(targetPath)}${overwrite}`,
    );
    const xhr = new XMLHttpRequest();
    xhr.open("POST", url);
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
    // W0a token 收尾：token 非空才注入，空值不加头（URL 也保持无 token）。
    const token = getAuthToken();
    if (token !== null) {
      xhr.setRequestHeader("Authorization", `Bearer ${token}`);
    }
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable) {
        onProgress?.(event.loaded, event.total);
      }
    };
    xhr.onerror = () => reject(new ApiError("上传失败（网络错误）", 0, ""));
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          resolve(JSON.parse(xhr.responseText) as UploadResult);
        } catch {
          resolve({ ok: false, message: "服务端返回了无法解析的响应" });
        }
      } else {
        reject(
          new ApiError(
            `上传失败：HTTP ${xhr.status}`,
            xhr.status,
            xhr.responseText,
          ),
        );
      }
    };
    xhr.send(file);
  });
}

/**
 * 下载地址（`GET /api/files/download?path=`），供 `<a download>` 直接使用。
 *
 * 取舍记录（W0a token 收尾）：`<a download>` 无法携带 `Authorization` 头，而
 * `server.expected_token` 非空时下载同样走 `auth_ok`。按 `files.rs:20`「头或
 * `?token=` 均可」的语义，这里选择 **query 传 token**——代价是 token 可能进入
 * 浏览器历史与服务端日志（泄漏面），header 方式无此问题；后端同样接受 header，
 * 将来若改为 fetch+blob 下载，应切回 `Authorization` 头。`getAuthToken()` 返回
 * null（未配置 token）时不追加任何参数，URL 与注入前完全一致。
 */
export const downloadUrl = (path: string): string => {
  const token = getAuthToken();
  // URL 已带 `?path=`，追加参数一律用 `&`；token 过滤空串已由 getAuthToken 保证。
  const tokenQuery =
    token === null ? "" : `&token=${encodeURIComponent(token)}`;
  return apiUrl(`files/download?path=${encodeURIComponent(path)}${tokenQuery}`);
};
