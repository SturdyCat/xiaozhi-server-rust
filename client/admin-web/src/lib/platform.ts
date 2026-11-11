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
 * 读取桌面壳能力信息。
 * - 浏览器环境（isTauri 为假）：直接返回 Web 兜底值，绝不调用 invoke。
 * - Tauri Local origin（内嵌 SPA，tauri://localhost）：返回真实值。
 * - Tauri Remote origin（生产服务器页面 / tauri dev 的 Vite URL）：
 *   invoke 会被 ACL 拒绝。此处**抛错**让调用方（TestbenchPage）显式提示，
 *   而不是用 platform:"web" 制造「通道正常」的误导。调用方必须自行 try/catch。
 */
export async function getCapabilities(): Promise<AppCapabilities> {
  if (!isTauri()) {
    return { ...WEB_CAPABILITIES };
  }
  try {
    return await invoke<AppCapabilities>("app_capabilities");
  } catch (cause: unknown) {
    // 远程页面 origin（生产服务器页面 / tauri dev）未被 capability 授权 remote.urls，
    // invoke 会被 ACL 拒绝。抛错让调用方（TestbenchPage）显式提示，
    // 而不是用 platform:"web" 制造「通道正常」的误导。调用方必须自行 try/catch。
    const reason = cause instanceof Error ? cause.message : String(cause);
    throw new Error(`IPC 通道不可用（远程 origin 未授权 remote.urls）：${reason}`);
  }
}
