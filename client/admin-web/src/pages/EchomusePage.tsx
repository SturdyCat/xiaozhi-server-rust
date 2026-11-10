import type { ReactElement } from "react";
import { PlaceholderPage } from "@/pages/PlaceholderPage";

export function EchomusePage(): ReactElement {
  return (
    <PlaceholderPage
      title="EchoMuse"
      description="官方 /em/ 页面后续以 iframe 方式接入。"
      route="/echomuse"
      origin="Rust server 官方页 /em/"
      notes={[
        "本阶段不实现 iframe；后续使用带尾斜杠的 /em/ 地址并附带 URL 参数。",
        "Vite dev proxy 与 SPA fallback 均已为 /em/ 预留排除口径，勿让前端路由占用该前缀。",
      ]}
    />
  );
}
