import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

/**
 * Tauri 在跨设备开发时注入的 HMR 主机名；未设置时固定监听 127.0.0.1，
 * 与 tauri.conf.json 的 devUrl 保持一致（避免 localhost 解析到 IPv6 ::1
 * 而 Tauri 走 IPv4 127.0.0.1 时连不上）。不要改成 true 监听所有网卡。
 */
const devHost: string | undefined = process.env.TAURI_DEV_HOST;

/** Rust server 的本地地址，可用 VITE_PROXY_TARGET 覆盖。 */
const proxyTarget: string = process.env.VITE_PROXY_TARGET || "http://127.0.0.1:8000";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  // Tauri CLI 需要读取 Vite 的完整日志输出。
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: devHost || "127.0.0.1",
    hmr: devHost
      ? { protocol: "ws" as const, host: devHost, port: 1421 }
      : { port: 1421 },
    watch: {
      ignored: ["**/src-tauri/**"],
    },
    // proxy 中间件先于 SPA history fallback 生效，因此 /em/（含尾斜杠）
    // 会原样转发到 Rust server，不会被 index.html 吞掉。
    proxy: {
      "/api": { target: proxyTarget, changeOrigin: true },
      "/ws": { target: proxyTarget, changeOrigin: true, ws: true },
      "/em": { target: proxyTarget, changeOrigin: true },
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: process.env.TAURI_ENV_PLATFORM === "darwin" ? "safari13" : "chrome105",
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
