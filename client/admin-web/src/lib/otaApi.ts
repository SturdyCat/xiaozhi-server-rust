/**
 * OTA 固件托管段（W10 / Config C7）的端点封装：清单 / 上传 / 删除。
 *
 * 契约以服务端为准（`server/src/device/firmware.rs` + `firmware/http.rs` 逐行核实）。
 * **混合鉴权是 C7 独有，勿一刀切**：
 * - **清单 `GET /api/ota/firmware` 无鉴权**（`list()` handler 不取 HeaderMap、不调
 *   `auth_ok`）——本文件的清单方法不写任何 401 分支；
 * - **上传 / 删除有鉴权**（`http.rs:50/:214` 调 `auth_ok`，头或 `?token=` 均可）；
 *   本文件按 W0a 口径走 `Authorization: Bearer`（`getAuthToken()` 非空才注入，不进
 *   URL/日志），**401 响应是纯文本 `"unauthorized"` 非 JSON**（`http.rs:51/:215`）。
 * - **feature 门控端点级 fail-closed**：整组 `#[cfg(feature = "native-plugins")]`
 *   （`ws/mod.rs:244-250` 经 `device::firmware::router()` 并入）——pure-base 下请求被
 *   SPA fallback 接走返回 **HTML 200**。清单**必须判形状** `{dir, count, entries[]}` +
 *   条目十字段，形状不符抛 [`OtaUnavailableError`]，**不得以 HTTP 200 判可用**。
 *   （注意：list 响应**没有 `ok` 键**——判形状，不是判 `ok===true`。）
 *
 * 上传 = **裸字节** `application/octet-stream`，坐标（project/model/board/version 覆盖
 * 参数）走 **query**（非 multipart 非 JSON）；服务端 64MB 上限（413「固件超过上限
 * 64MB」，纯文本）；同 功能+型号(+板型) 再次上传即覆盖旧包，不留历史。
 *
 * `metadata_from` **只按包内是否解析成功判定**（`http.rs:135` `parsed.is_some()`）：
 * 即便用户手填了覆盖参数，只要包内元数据解析成功仍会报 `"package"`（手填值优先，但
 * 标记不反映这一点）——提示文案必须留余地，**不写成无条件绝对判定**。
 *
 * 边界：`/api/ota/firmware/latest`（设备下载）与 `GET/POST /api/ota`（设备引导）是
 * **设备侧契约，本文件 0 调用**。错误体（400 / 404 / 413）是**纯文本中文**，不是
 * JSON `{ok:false}`——错误归一直接透出响应体文本。
 */

import { ApiError, apiUrl, getAuthToken, requestApi } from "@/lib/api";

/** OTA 固件清单读取失败（形状不符等结构性异常；feature 未编入时 SPA fallback HTML 200）。 */
export class OtaUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "OtaUnavailableError";
  }
}

/** 供页面判断是否走「无法读取固件清单」的失败分支。 */
export function isOtaUnavailable(error: unknown): boolean {
  return error instanceof OtaUnavailableError;
}

/** 把任意异常转成可展示的中文文案。 */
export function otaErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

/** 判断是否为普通 JSON 对象（数组与 null 不算）。 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// ============================================================
// 1. GET /api/ota/firmware（清单，无鉴权）
// ============================================================

/** 一条托管固件（`FirmwareMeta::to_json`，firmware.rs:82-95，十字段）。 */
export interface FirmwareEntry {
  /** 功能名（固件 esp_app_desc_t.project_name，如 xiaozhi）。 */
  project: string;
  /** 型号（镜像头 chip_id → esp32c3 / esp32s3…）。 */
  model: string;
  /** 可选板型；空 = 该功能+型号通用。 */
  board: string;
  version: string;
  size: number;
  sha256: string;
  /** 编译该固件的 IDF 版本。 */
  idfVer: string;
  /** 固件编译时刻（date time，来自 app 描述符）。 */
  builtAt: string;
  /** 上传时刻（Unix 秒）。 */
  uploadedAt: number;
  /** 旧版单固件（根目录 firmware.bin）：只列出、可删除，**不参与设备匹配**。 */
  legacy: boolean;
}

/** `GET /api/ota/firmware` 归一后的清单。 */
export interface OtaFirmwareList {
  /** 服务端存储目录（默认 /data/firmware；XIAOZHI_FIRMWARE_DIR 可覆盖）。 */
  dir: string;
  count: number;
  /** 已按上传时间倒序（服务端 scan() 排好）。 */
  entries: FirmwareEntry[];
}

/** 安全读字符串字段（缺失/类型不符返回空串）。 */
function str(raw: Record<string, unknown>, key: string): string {
  return typeof raw[key] === "string" ? (raw[key] as string) : "";
}

/** 安全读数字字段（缺失/非有限数返回 0）。 */
function num(raw: Record<string, unknown>, key: string): number {
  const value = raw[key];
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

/**
 * 读取托管固件清单。**不以 HTTP 200 判可用**（pure-base 下 HTML 200 也是 200）：
 * 形状不符（非对象 / entries 非数组 / 条目非对象）抛 [`OtaUnavailableError`]。
 * 空清单是正常态（尚未托管任何固件）。
 */
export async function getFirmwareList(): Promise<OtaFirmwareList> {
  let raw: unknown;
  try {
    raw = await requestApi<unknown>("ota/firmware");
  } catch (error) {
    if (error instanceof OtaUnavailableError) {
      throw error;
    }
    if (error instanceof ApiError) {
      throw new Error(`固件清单加载失败：${error.body.trim() || `HTTP ${error.status}`}`);
    }
    // HTML 200（pure-base 下 SPA fallback 接走）经 requestApi 的 JSON.parse 抛裸
    // SyntaxError：必须归一成 OtaUnavailableError，否则 pure-base 下 fail-closed
    // 解释失效、误导排障指引（与 soulMemoryApi.ts:90-92 同构；W10 缺陷单 major）。
    if (error instanceof SyntaxError) {
      throw new OtaUnavailableError(
        "固件清单加载失败：服务端返回了非 JSON 内容（通常是构建未启用 native-plugins，请求被管理页 HTML 回退接走）。按能力未启用处理，不伪装成空清单。",
      );
    }
    throw error instanceof Error ? error : new Error(String(error));
  }
  if (!isRecord(raw) || !Array.isArray(raw["entries"])) {
    throw new OtaUnavailableError(
      "固件清单加载失败：响应缺少 entries 数组（通常是构建未启用 native-plugins，请求被管理页 HTML 回退接走）。按能力未启用处理，不伪装成空清单。",
    );
  }
  const entries: FirmwareEntry[] = [];
  for (const item of raw["entries"]) {
    if (!isRecord(item)) {
      throw new OtaUnavailableError(
        "固件清单加载失败：entries 里出现了非对象条目（服务端返回异常）。按能力未启用处理，不伪装成空清单。",
      );
    }
    entries.push({
      project: str(item, "project"),
      model: str(item, "model"),
      board: str(item, "board"),
      version: str(item, "version"),
      size: num(item, "size"),
      sha256: str(item, "sha256"),
      idfVer: str(item, "idf_ver"),
      builtAt: str(item, "built_at"),
      uploadedAt: num(item, "uploaded_at"),
      legacy: item["legacy"] === true,
    });
  }
  return {
    dir: str(raw, "dir"),
    count: num(raw, "count"),
    entries,
  };
}

// ============================================================
// 2. POST /api/ota/firmware（上传，需鉴权，裸字节）
// ============================================================

/** 上传的手动覆盖参数（**URL query 覆盖参数，不是配置字段**；留空的键不进 query）。 */
export interface UploadOverrides {
  project?: string;
  model?: string;
  board?: string;
  version?: string;
}

/** `POST /api/ota/firmware` 归一后的响应（FirmwareMeta + metadata_from + path）。 */
export interface OtaUploadResult {
  project: string;
  model: string;
  board: string;
  version: string;
  size: number;
  sha256: string;
  /** `"package"` = 包内元数据解析成功（手填覆盖时也可能报它，见头注释留余地说明）。 */
  metadataFrom: string;
  /** 服务端落盘目录（展示用）。 */
  path: string;
}

/**
 * 上传固件包（裸字节 + query 坐标）。`onProgress` 回调真实百分比（XHR
 * `xhr.upload.onprogress`，非不确定态）。token 非空时注入 `Authorization: Bearer`
 * （不进 URL）；401 是纯文本 `"unauthorized"`，以 `ApiError.status === 401` 区分。
 */
export function uploadFirmware(
  file: File,
  overrides: UploadOverrides,
  onProgress?: (loaded: number, total: number) => void,
): Promise<OtaUploadResult> {
  return new Promise<OtaUploadResult>((resolve, reject) => {
    // 覆盖参数留空即不发：服务端从包内元数据解析（http.rs:153-161 空串/空白视为未给）。
    const params = new URLSearchParams();
    const put = (key: string, value: string | undefined): void => {
      const trimmed = (value ?? "").trim();
      if (trimmed !== "") {
        params.set(key, trimmed);
      }
    };
    put("project", overrides.project);
    put("model", overrides.model);
    put("board", overrides.board);
    put("version", overrides.version);
    const query = params.toString();
    const url = apiUrl(`ota/firmware${query === "" ? "" : `?${query}`}`);

    const xhr = new XMLHttpRequest();
    xhr.open("POST", url);
    xhr.setRequestHeader("Content-Type", "application/octet-stream");
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
        let parsed: unknown;
        try {
          parsed = JSON.parse(xhr.responseText);
        } catch {
          reject(
            new ApiError("上传失败：服务端返回了无法解析的响应", xhr.status, xhr.responseText),
          );
          return;
        }
        if (!isRecord(parsed)) {
          reject(new ApiError("上传失败：服务端响应不是 JSON 对象", xhr.status, xhr.responseText));
          return;
        }
        resolve({
          project: str(parsed, "project"),
          model: str(parsed, "model"),
          board: str(parsed, "board"),
          version: str(parsed, "version"),
          size: num(parsed, "size"),
          sha256: str(parsed, "sha256"),
          metadataFrom: str(parsed, "metadata_from"),
          path: str(parsed, "path"),
        });
        return;
      }
      // 错误体是纯文本（400 缺参 / 401 "unauthorized" / 413 超限），原样透出。
      reject(
        new ApiError(
          `上传失败：${xhr.responseText.trim() || `HTTP ${xhr.status}`}`,
          xhr.status,
          xhr.responseText,
        ),
      );
    };
    xhr.send(file);
  });
}

// ============================================================
// 3. POST /api/ota/firmware/delete（删除，需鉴权）
// ============================================================

/**
 * 删除一个托管条目（坐标走 query，与 DELETE 方法同一处理器 `http.rs:209-236`）。
 * 成功判 `{ok:true}`；400 缺参 / 404 未找到是纯文本中文，原样透出。
 */
export async function deleteFirmware(project: string, model: string, board: string): Promise<void> {
  const query = `project=${encodeURIComponent(project)}&model=${encodeURIComponent(model)}&board=${encodeURIComponent(board)}`;
  let raw: unknown;
  try {
    raw = await requestApi<unknown>(`ota/firmware/delete?${query}`, {
      method: "POST",
    });
  } catch (error) {
    if (error instanceof ApiError) {
      throw new Error(`删除失败：${error.body.trim() || `HTTP ${error.status}`}`);
    }
    throw error instanceof Error ? error : new Error(String(error));
  }
  if (!isRecord(raw) || raw["ok"] !== true) {
    throw new Error("删除失败：服务端未确认删除（响应缺少 ok:true），不视为已删除。");
  }
}
