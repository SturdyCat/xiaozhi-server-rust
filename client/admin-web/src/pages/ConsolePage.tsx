import type { ReactElement } from "react";
import { PlaceholderPage } from "@/pages/PlaceholderPage";

export function ConsolePage(): ReactElement {
  return (
    <PlaceholderPage
      title="控制台"
      description="实时日志、WebSocket 会话与调试信息。"
      route="/console"
      origin="Kuikly adminCore · 调试控制台"
      notes={["WebSocket 走同源 /ws；开发期 Vite proxy 已开启 ws: true 转发。"]}
    />
  );
}
