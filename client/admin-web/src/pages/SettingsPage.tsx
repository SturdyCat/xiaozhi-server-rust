import type { ReactElement } from "react";
import { PlaceholderPage } from "@/pages/PlaceholderPage";

export function SettingsPage(): ReactElement {
  return (
    <PlaceholderPage
      title="系统设置"
      description="管理员账号、通知与服务运行参数。"
      route="/settings"
      origin="Kuikly adminCore · 设置模块"
      notes={["本阶段不引入暗色主题切换，保持 shadcn 默认浅色与 GearUI 口径一致。"]}
    />
  );
}
