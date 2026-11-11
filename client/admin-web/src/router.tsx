import type { ComponentType, ReactElement } from "react";
import { Navigate, Outlet, Route, Routes } from "react-router-dom";
import { AppShell } from "@/components/AppShell";
import { visibleNavDescriptors } from "@/lib/nav";
import { isTauri } from "@/lib/platform";
import { AdvancedModeProvider } from "@/context/AdvancedModeContext";
import { ConfigDraftProvider } from "@/context/ConfigDraftContext";
import { ConfigPage } from "@/pages/ConfigPage";
import { ConsolePage } from "@/pages/ConsolePage";
import { DevicesPage } from "@/pages/DevicesPage";
import { DevicehubPage } from "@/pages/DevicehubPage";
import { EchomusePage } from "@/pages/EchomusePage";
import { FilesPage } from "@/pages/FilesPage";
import { OverviewPage } from "@/pages/OverviewPage";
import { PluginRuntimeRoute, PluginsPage } from "@/pages/PluginsPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { TestbenchPage } from "@/pages/TestbenchPage";

/** 每个导航描述符对应的页面组件；key 与 nav.ts 的 key 严格一致。 */
export const PAGE_COMPONENTS: Record<string, ComponentType> = {
  overview: OverviewPage,
  devices: DevicesPage,
  devicehub: DevicehubPage,
  config: ConfigPage,
  settings: SettingsPage,
  echomuse: EchomusePage,
  console: ConsolePage,
  files: FilesPage,
  plugins: PluginsPage,
  testbench: TestbenchPage,
};

/** 只在 Tauri 环境挂载硬件测试台路由，Web 环境不暴露硬件相关入口。 */
const tauriEnv: boolean = isTauri();

/**
 * /config 与 /settings 共享的 Provider 层：一份配置草稿跨两个视图
 * （对齐旧端 AdminPage.kt:71-72「同一 AdminPage 持有同一份 ConfigFormState
 * 跨两视图」——在 /settings 改动未保存切到 /config 不丢草稿，反之亦然）。
 * 只包住这两个路由，其它页面仍不无端拉取配置（离开配置域即卸载、丢弃草稿）。
 */
function ConfigProvidersLayout(): ReactElement {
  return (
    <AdvancedModeProvider>
      <ConfigDraftProvider>
        <Outlet />
      </ConfigDraftProvider>
    </AdvancedModeProvider>
  );
}

export function AppRouter(): ReactElement {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<Navigate to="/overview" replace />} />
        {visibleNavDescriptors(tauriEnv).map((item) => {
          // config / settings 走下面的共享 Provider layout（draft 跨两视图），不在此生成。
          if (item.key === "config" || item.key === "settings") {
            return null;
          }
          const PageComponent: ComponentType = PAGE_COMPONENTS[item.key];
          return <Route key={item.key} path={item.key} element={<PageComponent />} />;
        })}
        <Route element={<ConfigProvidersLayout />}>
          <Route path="config" element={<ConfigPage />} />
          <Route path="settings" element={<SettingsPage />} />
        </Route>
        {/* 插件动态运行页：必须在 `path="*"` 兜底之前声明。 */}
        <Route path="plugin/:pluginId/:entryId" element={<PluginRuntimeRoute />} />
        <Route path="*" element={<Navigate to="/overview" replace />} />
      </Route>
    </Routes>
  );
}
