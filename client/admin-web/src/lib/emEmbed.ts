/**
 * `/em/` 官方页 iframe 嵌入 URL 契约（W12 / devicehub，R4 拍板：不 vendor，CDN 直连）。
 *
 * 契约权威：em/README.md:122-148 + em/static/host.js:16-23（+ host.test.cjs）。
 * URL 三形态中本波只用**嵌入单设备**形态：`/em/?embed=1&device=<encoded-id>&token=<encoded-token>`。
 *
 * ## 硬约束（逐条对应 README）
 *
 * - 路径**必须带尾斜杠** `/em/`：Rust ServeDir 未知路径回退主站 index.html，缺尾斜杠
 *   会在 `/em/` 下二次初始化（README.md:182-183）；
 * - `embed` 仅精确值 `1` 启用嵌入模式（host.js:19；缺省/`0`/`true` 保留独立模式）；
 * - `device`：**单次** `encodeURIComponent` 编码（宿主 URLSearchParams 解码一次、适配器
 *   请求设备端点时重新编码）；禁止把原始 `+`/`&`/`#` 直接拼入、禁止二次编码；
 * - `token`：可选、`encodeURIComponent`；host.js 写入 `API.token` 并以
 *   `Authorization: Bearer <token>` 请求 `/api/echomuse/*`——**传的是管理台令牌**
 *   （`getAuthToken()`），不是设备面 `X-Em-Token`（两套凭据勿混）。URL 含凭据 →
 *   iframe 必须 `referrerpolicy="no-referrer"`（em/index.html 已有 `no-referrer`
 *   meta，双保险）；
 * - `tab`：白名单六值 `status/activity/config/shell/updates/logs`（`shell` → 官方内部
 *   `console` 由 host.js 处理，前端只传 `shell`）；缺省/非法回退 `status`；
 *   **未审批设备忽略 tab**（继续显示官方审批表单）——调用方保证仅对已批准设备传 tab；
 * - 嵌入模式只读指定设备的记录 + `/runtime`（每 3s 轮询），不请求设备列表；缺少/空
 *   `device` 直接显示错误且不发设备请求；
 * - 父容器必须给 iframe 明确高度（README.md:147）。
 *
 * ## R4 拍板：CDN 直连的诚实提示
 *
 * 官方页第三方库/字体走 CDN（cdnjs + jsdelivr + gstatic，全 SRI），内网/离线环境会
 * 白屏（em/README.md:41 原文）。外层**无法**读到 iframe 内「CDN 失败」的真实原因
 * （跨文档不可读），只能在 iframe 外方常驻诚实文案——不伪装成功能故障。
 */

/** 管理台令牌（Bearer，供 host.js 请求 `/api/echomuse/*`）；来自 sessionStorage。 */
import { getAuthToken } from "@/lib/api";

/** `tab` 白名单六值（host.js:16-23）；`shell` 由 host.js 内部映射到官方 console。 */
export const EM_TAB_WHITELIST: readonly string[] = [
  "status",
  "activity",
  "config",
  "shell",
  "updates",
  "logs",
];

/** iframe 外常驻诚实文案（R4 不 vendor 的已知限制，README.md:41 口径）。 */
export const EM_EMBED_CDN_NOTICE =
  "嵌入页依赖第三方库与字体走公共 CDN（cdnjs / jsdelivr / fonts.gstatic）联网加载；" +
  "内网或离线环境下下方区域会空白——这是网络限制，不是功能故障。";

/** iframe sandbox 白名单（对齐旧端 EchoMuseOfficialViewHost.kt:38，无 window.open）。 */
export const EM_IFRAME_SANDBOX = "allow-scripts allow-same-origin allow-forms allow-downloads";

/** 嵌入 URL 选项。 */
export interface EmEmbedOptions {
  /** 设备 id；空串/空白直接抛错（缺 device 时 host.js 显示错误且不发请求——外层不发 URL）。 */
  device: string;
  /** 管理台令牌；null/空串时不带 token 参数（服务端 expected_token 为空时可不带）。 */
  token?: string | null;
  /** 页签；仅白名单内且设备已批准才带（未审批设备忽略 tab——调用方保证）。 */
  tab?: string | null;
}

/**
 * 拼装嵌入单设备 URL：`/em/?embed=1&device=<enc>[&token=<enc>][&tab=<tab>]`。
 * 尾斜杠必须；device/token 各**单次** `encodeURIComponent`；参数顺序与 README 一致。
 */
export function buildEmEmbedUrl(options: EmEmbedOptions): string {
  const device = options.device.trim();
  if (device === "") {
    throw new Error("嵌入官方管理页需要设备 id（缺少 device 参数时不发请求）");
  }
  let url = `/em/?embed=1&device=${encodeURIComponent(device)}`;
  const token = options.token?.trim() ?? "";
  if (token !== "") {
    url += `&token=${encodeURIComponent(token)}`;
  }
  const tab = options.tab?.trim() ?? "";
  if (tab !== "" && EM_TAB_WHITELIST.includes(tab)) {
    url += `&tab=${encodeURIComponent(tab)}`;
  }
  return url;
}

/**
 * 便捷封装：读取当前管理台令牌后拼装嵌入 URL（未配置令牌时不带 token 参数——
 * 服务端 `[server].expected_token` 为空时本就免鉴权，带了反而把空串写进 URL）。
 */
export function buildEmEmbedUrlWithAdminToken(options: Omit<EmEmbedOptions, "token">): string {
  return buildEmEmbedUrl({ ...options, token: getAuthToken() });
}
