/**
 * 配置工作区编排层（/config 与 /settings 共用）。
 *
 * 来源：Kuikly `ConfigWorkspaceViews.kt:224-230`（权威）——settings 是 Config 工作
 * 区的第二视图：没有语音工作流轨道，左侧全局段分组菜单、右侧复用同一套卡片、
 * 共用一份 draft、一次 `POST /api/config`，0 个 Settings 专属字段/端点/api。
 *
 * 本组件只负责编排（左菜单 + 右侧 Section 渲染 + SaveBar + 状态条 + 确认弹窗），
 * 一律不写 Section 内部逻辑；可见分区由 `sectionIds` 白名单过滤：
 * - `/config`（ConfigPage）传 18 段全量，行为与拆分前完全一致；
 * - `/settings`（SettingsPage）传 6 全局段（overview/presets/aiui/server/tuya/ota），
 *   左菜单分组沿用 `SECTION_GROUPS`（pinned「总览」+ service「服务与接入」，
 *   过滤后恰为旧端 `ConfigSections.globalGroups` 两分组，段序与 order 一致）。
 *
 * draft / advancedMode 两个 Provider **不在本组件挂载**：它们由 router.tsx 的
 * Provider layout 提供给 /config 与 /settings 两个路由（对应旧端 AdminPage.kt:71-72
 * 「同一 AdminPage 持有同一份 ConfigFormState 跨两视图」）——在任一视图的改动
 * 未保存时切换到另一视图不丢草稿；离开 /config 与 /settings 则随 layout 卸载，
 * 与「Provider 只在配置域挂载、其它页面不无端拉取配置」的既有边界一致。
 */

import { useEffect, useState, type ComponentType, type ReactElement } from "react";
import { useNavigate } from "react-router-dom";
import { CircleAlert, LoaderCircle } from "lucide-react";
import { ConfirmDialog } from "@/components/feedback/ConfirmDialog";
import { useToast } from "@/components/feedback/ToastHost";
import { ConfigOverviewSection } from "@/components/config/ConfigOverviewSection";
import { ConfigPresetsSection } from "@/components/config/ConfigPresetsSection";
import { AIUISection } from "@/components/config/AIUISection";
import { AsrSection } from "@/components/config/AsrSection";
import { AudioSection } from "@/components/config/AudioSection";
import { CommandSection } from "@/components/config/CommandSection";
import { EchomuseSection } from "@/components/config/EchomuseSection";
import { LlmSection } from "@/components/config/LlmSection";
import { MemorySection } from "@/components/config/MemorySection";
import { McpSection } from "@/components/config/McpSection";
import { NluSection } from "@/components/config/NluSection";
import { OtaSection } from "@/components/config/OtaSection";
import { PlaybackSection } from "@/components/config/PlaybackSection";
import { ServerSection } from "@/components/config/ServerSection";
import { SoulSection } from "@/components/config/SoulSection";
import { TtsSection } from "@/components/config/TtsSection";
import { TuyaSection } from "@/components/config/TuyaSection";
import { VadSection } from "@/components/config/VadSection";
import { WakeupSection } from "@/components/config/WakeupSection";
import { SaveBar } from "@/components/config/SaveBar";
import { SectionCard } from "@/components/config/SectionCard";
import {
  SECTION_GROUPS,
  findSection,
  sectionsOf,
  type ConfigSection,
} from "@/components/config/configSections";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useAdvancedMode } from "@/context/AdvancedModeContext";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import { getConfigMeta, saveImpactOf } from "@/lib/configApi";
import { useApiQuery } from "@/lib/useApiQuery";
import { useConfigSchema } from "@/lib/useConfigSchema";
import { cn, formatDateTime } from "@/lib/utils";

export interface ConfigWorkspaceProps {
  /** 页头 H2 标题（旧端：流程节点视图=「节点管理」，全局段视图=「全局配置」）。 */
  readonly title: string;
  /** 页头描述文案。 */
  readonly description: string;
  /**
   * 可见分区 id 白名单：左菜单与右侧渲染都按它过滤。白名单外分区既不出现在
   * 菜单里也不会被渲染；`onJump` 目标落在白名单外时按 `jumpFallbackRoute` 回退。
   */
  readonly sectionIds: readonly string[];
  /**
   * `onJump` 目标在白名单外时的回退路由（例如 /settings 视图里能力总览的
   * 「去配置」指向流程段——无深链参数机制，诚实回退 /config 首页，对齐 W12
   * devicehub 齿轮弹层先例）；不传则目标不可达时不动作。
   */
  readonly jumpFallbackRoute?: string;
}

/** 未迁移分区的占位卡（当前 18 段已全迁移，作为 findSection 命中但无组件时的防御分支保留）。 */
function SectionPlaceholder({ section }: { section: ConfigSection }): ReactElement {
  return (
    <SectionCard title={section.label}>
      <div className="space-y-2 text-sm text-muted-foreground">
        <p>该分区尚未迁移到新管理台（W4–W11 分批推进）。</p>
        <p className="font-mono text-xs">{section.id}</p>
      </div>
    </SectionCard>
  );
}

/** audio 分区是复合视图：音频主表单 + 页尾 playback（C1 口径，保持不变）。 */
function AudioWithPlayback(): ReactElement {
  return (
    <>
      <AudioSection />
      <PlaybackSection />
    </>
  );
}

/**
 * 分区 id → 渲染组件。overview 消费 `onJump`，其余组件忽略该 prop；
 * 不得在此改 Section 内部行为（11 条诚实化文案锚点已由 W3–W10 落地）。
 */
const SECTION_COMPONENTS: Record<string, ComponentType<{ onJump: (sectionId: string) => void }>> = {
  overview: ConfigOverviewSection,
  presets: ConfigPresetsSection,
  vad: VadSection,
  asr: AsrSection,
  nlu: NluSection,
  llm: LlmSection,
  tts: TtsSection,
  command: CommandSection,
  wakeup: WakeupSection,
  soul: SoulSection,
  memory: MemorySection,
  server: ServerSection,
  aiui: AIUISection,
  audio: AudioWithPlayback,
  echomuse: EchomuseSection,
  tuya: TuyaSection,
  mcp: McpSection,
  // ota 是纯动作面板：不消费 SaveBar 的 dirtyCount / saving / hot / revision 语义。
  ota: OtaSection,
};

export function ConfigWorkspace({
  title,
  description,
  sectionIds,
  jumpFallbackRoute,
}: ConfigWorkspaceProps): ReactElement {
  const { advancedMode, toggle } = useAdvancedMode();
  const {
    loading,
    error,
    // 非致命提示（例如「已保存但回读失败」）：必须渲染，否则密钥「已配置」状态过期而用户无感。
    warning: draftWarning,
    conflict,
    revision,
    dirtyCount,
    dirtyPaths,
    saving,
    save,
    discard,
    reload,
    resolveConflict,
    lastResult,
  } = useConfigDraft();
  const { show } = useToast();
  const { isAdvancedField } = useConfigSchema();
  const metaQuery = useApiQuery(getConfigMeta);
  const navigate = useNavigate();

  const [active, setActive] = useState<string>(sectionIds[0] ?? "overview");
  const [reloadConfirmOpen, setReloadConfirmOpen] = useState(false);
  const [saveConfirmOpen, setSaveConfirmOpen] = useState(false);
  const [lastLoadedAt, setLastLoadedAt] = useState<number | null>(null);

  // 每次成功加载/保存（revision 变化）都刷新「最后更新」时间。
  useEffect(() => {
    if (!loading && revision !== null) {
      setLastLoadedAt(Date.now());
    }
  }, [loading, revision]);

  const meta = metaQuery.data;
  const metaTitle =
    [meta?.backend, meta?.loaded_from]
      .filter((item) => item !== null && item !== undefined)
      .join(" · ") || undefined;
  // 两条非致命提示合并展示：草稿层的「已保存但回读失败」优先（信息更新），
  // 其次是「配置文件来源信息读取失败」（不影响配置本身的加载与保存）。
  const warning =
    draftWarning ??
    (metaQuery.error === null ? null : `配置文件来源信息读取失败：${metaQuery.error}`);

  const doReload = (): void => {
    setReloadConfirmOpen(false);
    void reload();
    void metaQuery.refetch();
  };

  const handleReloadClick = (): void => {
    if (dirtyCount > 0) {
      setReloadConfirmOpen(true);
      return;
    }
    doReload();
  };

  // 机制④：简洁模式下，若本次改动落在服务端标记为「高级」的字段上，保存前列出路径请用户确认。
  // 注意：这里的「高级」是服务端元数据判定，**不代表字段在界面上被隐藏**
  // （例如 playback.mode 在简洁模式下仍常显）。别因文案误解把这套确认逻辑删掉。
  const advancedDirtyPaths = dirtyPaths.filter((path) => isAdvancedField(path));
  const needsAdvancedConfirm = !advancedMode && advancedDirtyPaths.length > 0;

  const doSave = async (): Promise<void> => {
    const result = await save();
    // 无改动 / 冲突 / 失败都返回 null：冲突时不弹「已保存」。
    if (result === null) {
      return;
    }
    show(result.message?.trim() ? result.message : "已保存", "success");
  };

  const handleSave = (): void => {
    if (needsAdvancedConfirm) {
      setSaveConfirmOpen(true);
      return;
    }
    void doSave();
  };

  const lastImpact = lastResult === null ? null : saveImpactOf(lastResult);
  const hot = lastImpact?.hot;
  const currentSection = findSection(active);
  const ActiveSection = SECTION_COMPONENTS[active];

  // 能力总览的「去配置」跳转：白名单内直接切分区；白名单外（/settings 视图指向
  // 流程段）无深链参数机制，按 `jumpFallbackRoute` 诚实回退，绝不渲染空白分区。
  const jumpTo = (sectionId: string): void => {
    if (sectionIds.includes(sectionId)) {
      setActive(sectionId);
      return;
    }
    if (jumpFallbackRoute !== undefined) {
      navigate(jumpFallbackRoute);
    }
  };

  return (
    <div className="mx-auto w-full max-w-6xl space-y-6">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="space-y-1">
          <h2 className="text-2xl font-semibold tracking-tight">{title}</h2>
          <p className="text-sm text-muted-foreground">{description}</p>
        </div>
        <div className="flex flex-wrap items-start gap-4">
          <div className="flex items-center gap-2">
            <span className="text-xs text-muted-foreground">{advancedMode ? "高级" : "简单"}</span>
            <Switch
              aria-label="切换高级模式"
              checked={advancedMode}
              onCheckedChange={() => toggle()}
            />
          </div>
          <Button variant="outline" size="sm" onClick={handleReloadClick}>
            重新加载
          </Button>
          <div className="max-w-[340px] text-right text-xs text-muted-foreground">
            <p>最后更新：{formatDateTime(lastLoadedAt)}</p>
            {meta?.config_path ? (
              <p className="truncate font-mono" title={metaTitle}>
                {meta.config_path}
              </p>
            ) : (
              <p className="truncate">
                {metaQuery.loading ? "读取配置文件路径…" : "配置文件路径未知"}
              </p>
            )}
            {meta?.persistent === false ? (
              <p className="text-amber-700">未持久化，重建容器会丢失</p>
            ) : null}
          </div>
        </div>
      </header>

      <div className="flex gap-6">
        <aside className="w-[220px] shrink-0">
          <nav className="space-y-4">
            {SECTION_GROUPS.map((group) => {
              // 白名单过滤：/settings 视图下各组只剩命中的全局段；
              // 段内顺序仍按 configSections 的 order 升序，不另起清单。
              const items = sectionsOf(group.id).filter((section) =>
                sectionIds.includes(section.id),
              );
              if (items.length === 0) {
                return null;
              }
              return (
                <div key={group.id} className="space-y-1">
                  <p className="px-3 text-xs font-medium text-muted-foreground">{group.label}</p>
                  {items.map((section) => (
                    <button
                      key={section.id}
                      type="button"
                      onClick={() => setActive(section.id)}
                      className={cn(
                        "w-full rounded-md px-3 py-2 text-left text-sm transition-colors",
                        active === section.id
                          ? "bg-accent font-medium text-accent-foreground"
                          : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                      )}
                    >
                      {section.label}
                    </button>
                  ))}
                </div>
              );
            })}
          </nav>
        </aside>

        <div className="min-w-0 flex-1 space-y-2">
          {loading ? (
            <div className="flex items-center gap-2 rounded-md border border-border bg-muted/40 px-3 py-2 text-sm text-muted-foreground">
              <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
              <span>加载配置中…</span>
            </div>
          ) : null}

          {/* 错误条与冲突条职责分离：冲突文案只在下面那条显示，这里不再用 !conflict 抑制，
              否则冲突之后的 reload 失败 / 400 / 500 都会对用户完全不可见。 */}
          {error !== null ? (
            <div
              role="alert"
              className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
            >
              <span className="flex min-w-0 items-center gap-2">
                <CircleAlert aria-hidden="true" className="size-4 shrink-0" />
                <span className="break-words">{error}</span>
              </span>
              <Button variant="outline" size="sm" onClick={() => void reload()}>
                重试
              </Button>
            </div>
          ) : null}

          {warning !== null ? (
            <div className="flex items-start gap-2 rounded-md border border-amber-600/30 bg-amber-50 px-3 py-2 text-sm text-amber-800">
              <CircleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
              <span className="break-words">{warning}</span>
            </div>
          ) : null}

          {conflict ? (
            <div
              role="alert"
              className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
            >
              <div className="space-y-1">
                <p>配置已被其他窗口/端修改，本次保存已取消（避免覆盖别人的改动）。</p>
                <p className="text-xs text-destructive/80">
                  你当前的修改仍保留在页面上，可先复制再重新加载。
                </p>
              </div>
              <Button variant="outline" size="sm" onClick={() => void resolveConflict()}>
                重新加载并放弃本地改动
              </Button>
            </div>
          ) : null}

          {ActiveSection ? (
            <ActiveSection onJump={jumpTo} />
          ) : currentSection ? (
            <SectionPlaceholder section={currentSection} />
          ) : null}
        </div>
      </div>

      <SaveBar
        dirtyCount={dirtyCount}
        saving={saving}
        hot={hot}
        ready={revision !== null}
        onSave={handleSave}
        onDiscard={discard}
        message={lastResult?.message}
      />

      <ConfirmDialog
        open={reloadConfirmOpen}
        title="重新加载配置"
        description={`将放弃 ${dirtyCount} 处未保存的修改，并重新从服务端读取配置。`}
        confirmText="重新加载"
        destructive
        onConfirm={doReload}
        onCancel={() => setReloadConfirmOpen(false)}
      />

      <ConfirmDialog
        open={saveConfirmOpen}
        title="简洁模式下保存高级字段"
        description={`本次改动含高级字段（${advancedDirtyPaths.join("、")}），请确认保存？`}
        confirmText="确认保存"
        onConfirm={() => {
          setSaveConfirmOpen(false);
          void doSave();
        }}
        onCancel={() => setSaveConfirmOpen(false)}
      />
    </div>
  );
}
