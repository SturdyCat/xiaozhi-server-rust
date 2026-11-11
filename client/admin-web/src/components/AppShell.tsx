import type { ElementType, ReactElement } from "react";
import { useEffect, useState } from "react";
import {
  AudioLines,
  Cpu,
  FlaskConical,
  FolderOpen,
  LayoutDashboard,
  Network,
  Puzzle,
  Settings2,
  SlidersHorizontal,
  Terminal,
} from "lucide-react";
import { NavLink, Outlet } from "react-router-dom";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { getApiBaseUrl } from "@/lib/api";
import { visibleNavDescriptors } from "@/lib/nav";
import { getPluginsFull, pluginNavPath } from "@/lib/pluginsApi";
import { isTauri } from "@/lib/platform";
import { cn } from "@/lib/utils";

/** 导航图标；key 与 nav.ts 的 key 严格一致。 */
const NAV_ICONS: Record<string, ElementType> = {
  overview: LayoutDashboard,
  devices: Cpu,
  devicehub: Network,
  config: SlidersHorizontal,
  settings: Settings2,
  echomuse: AudioLines,
  console: Terminal,
  files: FolderOpen,
  plugins: Puzzle,
  testbench: FlaskConical,
};

/** 动态插件导航项（registration.menu 的 nav.main 行，插件组）。 */
interface PluginNavEntryView {
  /** 路由相对路径（plugin/:pluginId/:entryId）。 */
  key: string;
  label: string;
}

/**
 * 动态侧栏入口：拉一次插件清单，取 Active 插件的 nav.main 菜单行。
 * 静态基线 fail-open：清单不可达 / wasm 键缺失 / 回包异常时返回空数组，
 * 只渲染静态导航，绝不让动态投影污染或清空静态菜单（PluginSlots.kt 语义）。
 */
function usePluginNavEntries(): PluginNavEntryView[] {
  const [entries, setEntries] = useState<PluginNavEntryView[]>([]);
  useEffect(() => {
    let cancelled = false;
    getPluginsFull()
      .then((data) => {
        if (cancelled || data.wasm === null) {
          return;
        }
        // 合并口径与 PluginSlots.mergeNav 一致：按 order 升序（coerceIn(0,999)），
        // 同序号按 pluginId / entryId 稳定排序，落在「插件」组。
        const dynamic = data.wasm.navEntries
          .slice()
          .sort(
            (a, b) =>
              a.order - b.order ||
              a.pluginId.localeCompare(b.pluginId) ||
              a.entryId.localeCompare(b.entryId),
          )
          .map((entry) => ({
            key: pluginNavPath(entry),
            label: entry.label,
          }));
        if (!cancelled) {
          setEntries(dynamic);
        }
      })
      .catch(() => {
        // 静态基线 fail-open：清单加载失败不报错、不加动态入口。
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return entries;
}

/** 浅色侧边栏 + 内容区骨架，全部使用 shadcn/ui 组件。 */
export function AppShell(): ReactElement {
  const tauriEnv: boolean = isTauri();
  const items = visibleNavDescriptors(tauriEnv);
  const pluginEntries = usePluginNavEntries();
  const apiBaseUrl: string = getApiBaseUrl();

  return (
    <div className="flex min-h-screen bg-background text-foreground">
      <aside className="flex w-64 shrink-0 flex-col border-r border-border bg-sidebar text-sidebar-foreground">
        <div className="flex h-16 shrink-0 items-center gap-3 px-6">
          <img src="/logo.png" alt="小智" className="size-9 shrink-0 rounded-lg object-cover" />
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold">小智管理台</p>
            <p className="truncate text-xs text-muted-foreground">Admin Web 骨架</p>
          </div>
        </div>
        <Separator />
        <nav className="flex-1 space-y-1 overflow-y-auto p-3">
          {items.map((item) => {
            const Icon: ElementType = NAV_ICONS[item.key];
            return (
              <NavLink
                key={item.key}
                to={`/${item.key}`}
                className={({ isActive }) =>
                  cn(
                    "flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                    isActive
                      ? "bg-sidebar-accent text-sidebar-accent-foreground"
                      : "text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                  )
                }
              >
                <Icon className="size-4 shrink-0" />
                <span className="truncate">{item.label}</span>
                {item.tauriOnly ? (
                  <Badge variant="outline" className="ml-auto shrink-0">
                    桌面
                  </Badge>
                ) : null}
              </NavLink>
            );
          })}
          {/* 动态插件组（fail-open：清单不可用时不出现，静态基线不受影响）。 */}
          {pluginEntries.length > 0 ? (
            <div className="pt-2">
              <p className="px-3 pb-1 text-xs font-medium text-muted-foreground">插件</p>
              {pluginEntries.map((entry) => (
                <NavLink
                  key={entry.key}
                  to={`/${entry.key}`}
                  className={({ isActive }) =>
                    cn(
                      "flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                      isActive
                        ? "bg-sidebar-accent text-sidebar-accent-foreground"
                        : "text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                    )
                  }
                >
                  <Puzzle className="size-4 shrink-0" />
                  <span className="truncate">{entry.label}</span>
                </NavLink>
              ))}
            </div>
          ) : null}
        </nav>
        <Separator />
        <div className="flex shrink-0 items-center justify-between gap-2 p-4">
          <span className="text-xs text-muted-foreground">运行环境</span>
          <Badge variant="secondary">{tauriEnv ? "Tauri 桌面端" : "Web 浏览器"}</Badge>
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-16 shrink-0 items-center justify-between gap-4 border-b border-border px-6">
          <div className="min-w-0">
            <h1 className="truncate text-base font-semibold">小智服务端管理台</h1>
            <p className="truncate text-xs text-muted-foreground">
              React + shadcn/ui + Tauri 2 · 开发环境骨架
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <Badge variant="outline" className="hidden sm:inline-flex">
              API {apiBaseUrl}
            </Badge>
            <Button variant="outline" size="sm" onClick={() => window.location.reload()}>
              刷新
            </Button>
          </div>
        </header>
        <main className="flex-1 bg-muted/30 p-6">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
