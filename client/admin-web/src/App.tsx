import type { ReactElement } from "react";
import { HashRouter } from "react-router-dom";
import { AppRouter } from "./router";

/**
 * 采用 HashRouter：静态托管（Rust ServeDir）与 Tauri 内嵌 SPA 都没有
 * 服务端 history fallback，hash 路由可保证刷新/深链接不 404。
 */
export default function App(): ReactElement {
  return (
    <HashRouter>
      <AppRouter />
    </HashRouter>
  );
}
