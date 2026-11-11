/**
 * 配置页「能力总览」（C0，只读）。
 *
 * 数据来源：
 * - `GET /api/health/engines`：能力级 phase / active_impl / last_error / config_section；
 * - `GET /api/plugins`：能力是否启用（`enabled`），与 health 同源聚合；
 * - `GET /api/config/schema`：`app_sections`（非插件配置段）的生效档位。
 *
 * 这里不写任何配置，只提供「去配置」跳转（由父组件切换分区）。
 */

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { DataState } from "@/components/feedback/DataState";
import { HOT_LABEL, type SaveBarHot } from "@/components/config/SaveBar";
import { SectionCard } from "@/components/config/SectionCard";
import { findSection, type ConfigSection } from "@/components/config/configSections";
import { getEngines, type EngineCapability } from "@/lib/adminApi";
import { getConfigSchema, type ConfigHotLevel } from "@/lib/configApi";
import { useApiQuery } from "@/lib/useApiQuery";
import type { ReactElement } from "react";

export interface ConfigOverviewSectionProps {
  /** 点击「去配置」时切换到对应分区。 */
  onJump: (sectionId: string) => void;
}

/** 能力 id 与配置分区 id 不一致的少数情况（作为旧服务端缺 `config_section` 时的兜底）。 */
const CAPABILITY_SECTION_OVERRIDES: Record<string, string> = {
  fullchain: "aiui",
  firmware: "ota",
};

/**
 * 引擎 phase 的中文标签。
 *
 * 服务端取值是 `active` / `degraded` / `failed` / `disabled`（`plugins/host.rs`），
 * 原样渲染英文枚举会让中文界面出现 `failed` 这种裸值；未知值仍回落原文，
 * 同时把原始枚举放进 `title` 便于排查。
 */
const PHASE_LABEL: Record<string, string> = {
  active: "正常",
  degraded: "降级",
  failed: "失败",
  disabled: "未启用",
};

/**
 * 把能力映射到配置分区。
 *
 * 优先使用服务端 `config_section`（权威映射：fullchain→aiui、firmware→ota、
 * voices→null）；字段缺失时回落到本地同名单/兜底表。
 */
function sectionForCapability(capability: EngineCapability): ConfigSection | undefined {
  const fromServer: unknown = capability.config_section;
  if (typeof fromServer === "string" && fromServer !== "") {
    return findSection(fromServer);
  }
  const fallback = CAPABILITY_SECTION_OVERRIDES[capability.capability] ?? capability.capability;
  return findSection(fallback);
}

/** phase → Badge 配色；同时兼容服务端的 active/failed 与旧文案 ready/unavailable。 */
function phaseBadgeClass(phase: string): string {
  switch (phase) {
    case "active":
    case "ready":
      return "border-emerald-600/30 bg-emerald-50 text-emerald-700";
    case "degraded":
      return "border-amber-600/30 bg-amber-50 text-amber-700";
    case "failed":
    case "unavailable":
    case "error":
      return "border-destructive/30 bg-destructive/5 text-destructive";
    default:
      return "border-border bg-muted text-muted-foreground";
  }
}

/** hot 档位 → Badge 配色。 */
function hotBadgeClass(hot: ConfigHotLevel): string {
  if (hot === "live") {
    return "border-emerald-600/30 bg-emerald-50 text-emerald-700";
  }
  if (hot === "next_session") {
    return "border-amber-600/30 bg-amber-50 text-amber-700";
  }
  return "border-border bg-muted text-muted-foreground";
}

export function ConfigOverviewSection({ onJump }: ConfigOverviewSectionProps): ReactElement {
  const enginesQuery = useApiQuery(getEngines);
  const schemaQuery = useApiQuery(getConfigSchema);

  // 能力状态以 `/api/health/engines` 为准（phase 含运行期探测合并）；
  // `/api/plugins` 也带 enabled，但它的「未启用」与 phase=disabled 语义重叠，
  // 两处都取会让同一行出现两个「未启用」。这里只用 engines，省一次请求。
  const capabilities = enginesQuery.data?.capabilities ?? [];

  const healthLoading = enginesQuery.loading;
  const healthError = enginesQuery.error;
  const refetchHealth = (): void => {
    void enginesQuery.refetch();
  };

  const appSections = schemaQuery.data?.app_sections ?? [];

  return (
    <div className="space-y-4">
      <SectionCard
        title="能力健康"
        description="服务端按能力聚合的运行状态（/api/health/engines，与 /api/plugins 同源）。"
      >
        <DataState
          loading={healthLoading}
          error={healthError}
          empty={capabilities.length === 0}
          emptyText="服务端未返回任何能力"
          onRetry={refetchHealth}
        >
          <div className="overflow-x-auto">
            <table className="w-full min-w-[560px] text-sm">
              <thead>
                <tr className="border-b border-border text-left text-xs text-muted-foreground">
                  <th className="py-2 pr-3 font-medium">能力</th>
                  <th className="py-2 pr-3 font-medium">当前实现</th>
                  <th className="py-2 pr-3 font-medium">状态</th>
                  <th className="py-2 text-right font-medium">操作</th>
                </tr>
              </thead>
              <tbody>
                {capabilities.map((capability) => {
                  const section = sectionForCapability(capability);
                  return (
                    <tr
                      key={capability.capability}
                      className="border-b border-border align-top last:border-0"
                    >
                      <td className="py-2 pr-3">
                        <div className="font-medium">{capability.display}</div>
                        {/* 「未启用」已由右侧 phase 徽标表达，这里不再重复一遍。 */}
                        <div className="font-mono text-xs text-muted-foreground">
                          {capability.capability}
                        </div>
                      </td>
                      <td className="py-2 pr-3 font-mono text-xs">
                        {capability.active_impl ?? "—"}
                      </td>
                      <td className="py-2 pr-3">
                        <Badge
                          variant="outline"
                          className={phaseBadgeClass(capability.phase)}
                          title={capability.last_error ?? `phase: ${capability.phase}`}
                        >
                          {PHASE_LABEL[capability.phase] ?? capability.phase}
                        </Badge>
                      </td>
                      <td className="py-2 text-right">
                        {section ? (
                          <Button variant="outline" size="sm" onClick={() => onJump(section.id)}>
                            去配置
                          </Button>
                        ) : (
                          <span className="text-xs text-muted-foreground">—</span>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </DataState>
      </SectionCard>

      <SectionCard
        title="生效档位"
        description="非插件配置段的生效时机；说明文案直接使用服务端 hot_hint 原话。"
      >
        <DataState
          loading={schemaQuery.loading}
          error={schemaQuery.error}
          empty={appSections.length === 0}
          emptyText="服务端未声明应用级配置段"
          onRetry={() => void schemaQuery.refetch()}
        >
          <div className="divide-y divide-border">
            {appSections.map((section) => (
              <div
                key={section.id}
                className="flex items-center justify-between gap-3 py-2 first:pt-0 last:pb-0"
              >
                <div className="min-w-0">
                  <p className="font-mono text-sm">{section.id}</p>
                  <p className="text-xs text-muted-foreground">{section.hot_hint}</p>
                </div>
                <Badge variant="outline" className={hotBadgeClass(section.hot)}>
                  {HOT_LABEL[section.hot as SaveBarHot] ?? section.hot}
                </Badge>
              </div>
            ))}
          </div>
        </DataState>
      </SectionCard>
    </div>
  );
}
