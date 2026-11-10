import { invoke, isTauri as tauriIsTauri } from "@tauri-apps/api/core";

/** 与 Rust 侧 commands::AppCapabilities 的 serde 输出保持一致。 */
export interface AppCapabilities {
  platform: string;
  tauri: boolean;
  version: string;
}

const WEB_CAPABILITIES: AppCapabilities = {
  platform: "web",
  tauri: false,
  version: "0.1.0",
};

/**
 * Tauri 2 能力检测（不是 UA 判断，同步返回 boolean）。
 *
 * 优先使用 `@tauri-apps/api/core` 官方 `isTauri()`（实现为
 * `!!globalThis.isTauri`，由 Tauri 2 init script 无条件注入）；再以
 * `__TAURI_INTERNALS__` 兜底双保险。`app.withGlobalTauri: false` 只关闭
 * `__TAURI__` 全局命名空间，不影响这两个注入点。
 */
export function isTauri(): boolean {
  if (tauriIsTauri()) {
    return true;
  }
  if (typeof window === "undefined") {
    return false;
  }
  return "__TAURI_INTERNALS__" in window;
}

/**
 * 读取桌面壳能力信息。浏览器环境直接返回 Web 兜底值，绝不调用 invoke；
 * Tauri 环境若命令未注册或调用失败也降级，保证页面不会因骨架命令崩溃。
 */
export async function getCapabilities(): Promise<AppCapabilities> {
  if (!isTauri()) {
    return { ...WEB_CAPABILITIES };
  }
  try {
    return await invoke<AppCapabilities>("app_capabilities");
  } catch {
    return { ...WEB_CAPABILITIES, tauri: true };
  }
}
