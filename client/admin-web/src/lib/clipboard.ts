/**
 * 统一复制入口：三级降级，任何情况下都不把复制失败抛给 UI。
 *
 * 背景：管理台常以 `http://LAN-IP:8000` 访问（非安全上下文），此时
 * `navigator.clipboard` 不存在；Tauri 生产默认加载的服务器页面又是远程
 * origin，未配置 `remote.urls` 时插件 IPC 会被 ACL 拒绝。因此三条通道都要试。
 *
 * 优先级：
 *   ① Tauri 原生插件（仅在 `isTauri()` 为真时尝试，IPC 失败即吞掉继续降级）；
 *   ② `navigator.clipboard.writeText`（浏览器安全上下文）；
 *   ③ 隐藏 textarea + `document.execCommand("copy")`（非安全上下文兜底）。
 *
 * 三级全部失败时只输出一条 console.warn 并正常 resolve：调用方无需 try/catch，
 * 也不会出现未处理的 Promise rejection 冒泡到 UI。
 */

import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { isTauri } from "@/lib/platform";

/**
 * 隐藏 textarea + execCommand 复制，返回是否成功；无论成功失败都会清理 DOM。
 *
 * @param text 待复制文本。
 * @returns `document.execCommand("copy")` 的返回值；异常时返回 false。
 */
function legacyCopy(text: string): boolean {
  if (typeof document === "undefined") {
    return false;
  }

  const previousActive: HTMLElement | null =
    document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const textarea: HTMLTextAreaElement = document.createElement("textarea");

  textarea.value = text;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.top = "0";
  textarea.style.left = "-9999px";
  textarea.style.opacity = "0";
  textarea.style.pointerEvents = "none";

  // 建元素/挂 DOM 也可能抛（例如 document.body 尚不存在）：copyText 的契约是
  // 「总是 resolve 一个 boolean」，这里不能把异常漏出去，否则调用方拿不到失败反馈。
  try {
    document.body.appendChild(textarea);
  } catch {
    return false;
  }
  try {
    textarea.focus();
    textarea.select();
    textarea.setSelectionRange(0, textarea.value.length);
    return document.execCommand("copy");
  } catch {
    return false;
  } finally {
    // 必须清理：占位 textarea 不能残留在 DOM 里，并尽量恢复原焦点。
    textarea.remove();
    if (previousActive !== null) {
      previousActive.focus();
    }
  }
}

/**
 * 把文本写入系统剪贴板，按 Tauri IPC → Clipboard API → execCommand 顺序降级。
 *
 * @param text 待复制文本；非字符串（JS 调用方）会先转成字符串。
 * @returns 是否至少有一个通道成功。调用方据此给用户反馈——静默失败会让
 *          「复制全部」看起来生效了，实际剪贴板没变。
 */
export async function copyText(text: string): Promise<boolean> {
  const value: string = typeof text === "string" ? text : String(text);

  // ① Tauri 原生插件。远程服务器页面是 remote origin，capabilities/default.json
  // 未配 remote.urls，ACL 会拒绝 plugin IPC；也可能插件尚未就绪。两种都吞掉继续降级。
  if (isTauri()) {
    try {
      await writeText(value);
      return true;
    } catch {
      // 刻意不抛出：降级到浏览器通道。
    }
  }

  // ② 浏览器 Clipboard API：仅 localhost / https 等安全上下文存在。
  try {
    const clipboard: Clipboard | undefined =
      typeof navigator === "undefined" ? undefined : navigator.clipboard;
    if (clipboard !== undefined && typeof clipboard.writeText === "function") {
      await clipboard.writeText(value);
      return true;
    }
  } catch {
    // 用户拒绝授权 / 非安全上下文 / 权限策略：降级到 execCommand。
  }

  // ③ 隐藏 textarea 兜底（局域网 http:// 非安全上下文的主要通道）。
  const copied: boolean = legacyCopy(value);
  if (!copied) {
    console.warn("[copyText] 三种复制通道均不可用，本次复制失败");
  }
  return copied;
}
