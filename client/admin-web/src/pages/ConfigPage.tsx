import type { ReactElement } from "react";
import { PlaceholderPage } from "@/pages/PlaceholderPage";

export function ConfigPage(): ReactElement {
  return (
    <PlaceholderPage
      title="配置管理"
      description="服务端参数与配置热生效档位。"
      route="/config"
      origin="Kuikly adminCore · 配置中心"
      notes={["配置热生效档位由注册表元数据唯一声明，迁移时不要另起一套口径。"]}
    />
  );
}
