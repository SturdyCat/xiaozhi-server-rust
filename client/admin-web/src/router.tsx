import type { ComponentType, ReactElement } from "react";
import { Navigate, Route, Routes } from "react-router-dom";
import { AppShell } from "@/components/AppShell";
import { visibleNavDescriptors } from "@/lib/nav";
import { isTauri } from "@/lib/platform";
import { ConfigPage } from "@/pages/ConfigPage";
import { ConsolePage } from "@/pages/ConsolePage";
import { DevicesPage } from "@/pages/DevicesPage";
import { EchomusePage } from "@/pages/EchomusePage";
import { FilesPage } from "@/pages/FilesPage";
import { OverviewPage } from "@/pages/OverviewPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { TestbenchPage } from "@/pages/TestbenchPage";

/** 每个导航描述符对应的页面组件；key 与 nav.ts 的 key 严格一致。 */
export const PAGE_COMPONENTS: Record<string, ComponentType> = {
  overview: OverviewPage,
  devices: DevicesPage,
  config: ConfigPage,
  settings: SettingsPage,
  echomuse: EchomusePage,
  console: ConsolePage,
  files: FilesPage,
  testbench: TestbenchPage,
};

/** 只在 Tauri 环境挂载硬件测试台路由，Web 环境不暴露硬件相关入口。 */
const tauriEnv: boolean = isTauri();

export function AppRouter(): ReactElement {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<Navigate to="/overview" replace />} />
        {visibleNavDescriptors(tauriEnv).map((item) => {
          const PageComponent: ComponentType = PAGE_COMPONENTS[item.key];
          return (
            <Route
              key={item.key}
              path={item.key}
              element={<PageComponent />}
            />
          );
        })}
        <Route path="*" element={<Navigate to="/overview" replace />} />
      </Route>
    </Routes>
  );
}
