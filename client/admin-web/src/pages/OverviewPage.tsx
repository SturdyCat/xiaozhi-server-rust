import { type ReactElement, type ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { cn, formatDateTime, formatRelative, formatUptime } from "@/lib/utils";
import { useApiQuery } from "@/lib/useApiQuery";
import {
  getDevices,
  getEngines,
  getGeo,
  getHealth,
  getPlugins,
  getStats,
  type EngineCapability,
} from "@/lib/adminApi";

/** 按 phase / enabled 决定能力 Badge 的颜色与文案。 */
function phaseBadgeClass(phase: string, enabled: boolean): string {
  if (!enabled) {
    return "bg-gray-500/15 text-gray-600";
  }
  switch (phase) {
    case "active":
      return "bg-emerald-500/15 text-emerald-700";
    case "degraded":
      return "bg-yellow-500/15 text-yellow-700";
    case "failed":
      return "bg-red-500/15 text-red-700";
    default:
      return "bg-gray-500/15 text-gray-600";
  }
}

/** 单值统计卡。 */
function StatCard({
  label,
  value,
  hint,
}: {
  label: string;
  value: ReactNode;
  hint?: string;
}) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardDescription>{label}</CardDescription>
        <CardTitle className="text-2xl font-semibold">{value}</CardTitle>
      </CardHeader>
      {hint ? (
        <CardContent className="text-xs text-muted-foreground">{hint}</CardContent>
      ) : null}
    </Card>
  );
}

/** 能力健康行：display 名 + 状态 Badge + 可选错误文案。 */
function CapabilityRow({ cap }: { cap: EngineCapability }) {
  return (
    <div className="flex items-start justify-between gap-3 rounded-md border px-3 py-2">
      <div className="min-w-0">
        <div className="truncate font-medium">{cap.display}</div>
        {cap.last_error ? (
          <div
            className="truncate text-xs text-destructive"
            title={cap.last_error}
          >
            {cap.last_error}
          </div>
        ) : null}
      </div>
      <Badge className={cn("shrink-0", phaseBadgeClass(cap.phase, cap.enabled))}>
        {cap.enabled ? cap.phase : "disabled"}
      </Badge>
    </div>
  );
}

export function OverviewPage(): ReactElement {
  const health = useApiQuery(getHealth);
  const stats = useApiQuery(getStats);
  const engines = useApiQuery(getEngines);
  const devices = useApiQuery(getDevices);
  const geo = useApiQuery(getGeo);
  const plugins = useApiQuery(getPlugins);

  const queries = [health, stats, engines, devices, geo, plugins];
  const anyLoading = queries.some((q) => q.loading);
  const firstError = queries.find((q) => q.error !== null)?.error ?? null;
  const lastUpdated =
    Math.max(0, ...queries.map((q) => q.updatedAt ?? 0)) || null;

  const handleRefresh = (): void => {
    health.refetch();
    stats.refetch();
    engines.refetch();
    devices.refetch();
    geo.refetch();
    plugins.refetch();
  };

  const onlineDevices = devices.data?.devices.filter((d) => d.online) ?? [];
  const serviceOk = (health.data?.status ?? "") === "ok";

  return (
    <div className="mx-auto w-full max-w-6xl space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="space-y-1">
          <h2 className="text-2xl font-semibold tracking-tight">总览</h2>
          <p className="text-sm text-muted-foreground">
            设备在线状态、会话统计与服务运行概况。
          </p>
        </div>
        <div className="flex items-center gap-3">
          <span className="text-xs text-muted-foreground">
            最后更新：{formatDateTime(lastUpdated)}
          </span>
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            disabled={anyLoading}
          >
            {anyLoading ? "刷新中…" : "刷新"}
          </Button>
        </div>
      </div>
      <Separator />

      {firstError !== null ? (
        <div className="flex items-center justify-between gap-3 rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          <span>加载失败：{firstError}</span>
          <Button variant="outline" size="sm" onClick={handleRefresh}>
            重试
          </Button>
        </div>
      ) : null}

      {anyLoading ? (
        <div className="text-sm text-muted-foreground">加载中…</div>
      ) : (
        <div className="space-y-6">
          <div className="grid grid-cols-2 gap-4 md:grid-cols-3 lg:grid-cols-4">
            <StatCard
              label="服务状态"
              value={
                <span className="flex items-center gap-2">
                  <span
                    className={cn(
                      "inline-block size-2.5 rounded-full",
                      serviceOk ? "bg-emerald-500" : "bg-gray-400",
                    )}
                  />
                  <span>{health.data?.status ?? "—"}</span>
                </span>
              }
            />
            <StatCard label="版本" value={health.data?.version ?? "—"} />
            <StatCard
              label="运行时间"
              value={formatUptime(stats.data?.uptime_ms ?? 0)}
            />
            <StatCard
              label="在线设备"
              value={`${devices.data?.online_count ?? 0} / ${devices.data?.count ?? 0}`}
            />
            <StatCard
              label="活跃会话"
              value={stats.data?.sessions.live ?? 0}
            />
            <StatCard
              label="对话数"
              value={stats.data?.conversations.retained ?? 0}
            />
            <StatCard
              label="消息轮次"
              value={stats.data?.turns.retained ?? 0}
            />
          </div>

          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            <Card>
              <CardHeader>
                <CardTitle>能力健康</CardTitle>
                <CardDescription>
                  引擎能力状态（active / degraded / failed / disabled）
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-2 text-sm">
                {engines.data && engines.data.capabilities.length > 0 ? (
                  engines.data.capabilities.map((cap) => (
                    <CapabilityRow key={cap.capability} cap={cap} />
                  ))
                ) : (
                  <p className="text-muted-foreground">无能力数据</p>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>在线设备</CardTitle>
                <CardDescription>当前在线的设备（脱机设备不在此列）</CardDescription>
              </CardHeader>
              <CardContent className="space-y-2 text-sm">
                {onlineDevices.length > 0 ? (
                  onlineDevices.map((d) => (
                    <div
                      key={d.device_id}
                      className="flex items-start justify-between gap-3 rounded-md border px-3 py-2"
                    >
                      <div className="min-w-0">
                        <div className="truncate font-mono text-xs">
                          {d.device_id}
                        </div>
                        <div className="text-xs text-muted-foreground">
                          {d.board} · {d.firmware_version}
                        </div>
                      </div>
                      <span className="shrink-0 text-xs text-muted-foreground">
                        {formatRelative(d.last_seen_ms)}
                      </span>
                    </div>
                  ))
                ) : (
                  <p className="text-muted-foreground">无在线设备</p>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>服务器出口</CardTitle>
                <CardDescription>服务端出口 IP 与地理位置</CardDescription>
              </CardHeader>
              <CardContent className="space-y-1 text-sm">
                <div className="font-mono">{geo.data?.ip ?? "—"}</div>
                <div className="text-muted-foreground">
                  {[geo.data?.country, geo.data?.province, geo.data?.city]
                    .filter(Boolean)
                    .join(" / ") || "—"}
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>插件能力</CardTitle>
                <CardDescription>已注册插件能力开关</CardDescription>
              </CardHeader>
              <CardContent className="space-y-2 text-sm">
                {plugins.data && plugins.data.capabilities.length > 0 ? (
                  plugins.data.capabilities.map((p) => (
                    <div
                      key={p.id}
                      className="flex items-center justify-between gap-3 rounded-md border px-3 py-2"
                    >
                      <span className="truncate font-mono text-xs">{p.id}</span>
                      <Badge
                        className={cn(
                          "shrink-0",
                          p.enabled
                            ? "bg-emerald-500/15 text-emerald-700"
                            : "bg-gray-500/15 text-gray-600",
                        )}
                      >
                        {p.enabled ? "已启用" : "已禁用"}
                      </Badge>
                    </div>
                  ))
                ) : (
                  <p className="text-muted-foreground">无插件数据</p>
                )}
              </CardContent>
            </Card>
          </div>
        </div>
      )}
    </div>
  );
}
