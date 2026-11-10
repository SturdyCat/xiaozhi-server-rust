import type { ReactElement } from "react";
import { PlaceholderPage } from "@/pages/PlaceholderPage";

export function FilesPage(): ReactElement {
  return (
    <PlaceholderPage
      title="文件管理"
      description="固件、音频与静态资源的上传和管理。"
      route="/files"
      origin="Kuikly adminCore · 资源模块"
      notes={["文件上传接口后续接入，不在本阶段暴露任何硬件动作。"]}
    />
  );
}
