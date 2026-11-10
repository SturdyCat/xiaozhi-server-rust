import type { ReactElement } from "react";
import { PlaceholderPage } from "@/pages/PlaceholderPage";

export function OverviewPage(): ReactElement {
  return (
    <PlaceholderPage
      title="总览"
      description="设备在线状态、会话统计与服务运行概况。"
      route="/overview"
      origin="Kuikly adminCore · 控制台首页"
      notes={["后续接入 /api 概览接口，统一由 @/lib/api 发起同源请求。"]}
    />
  );
}
