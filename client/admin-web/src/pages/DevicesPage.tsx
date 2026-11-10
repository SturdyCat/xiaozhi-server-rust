import type { ReactElement } from "react";
import { PlaceholderPage } from "@/pages/PlaceholderPage";

export function DevicesPage(): ReactElement {
  return (
    <PlaceholderPage
      title="设备管理"
      description="设备列表、绑定关系、在线状态与固件版本。"
      route="/devices"
      origin="Kuikly adminCore · 设备模块"
      notes={["设备实时状态后续走 /ws WebSocket 通道，地址由 @/lib/api 的 getWebSocketUrl() 推导。"]}
    />
  );
}
