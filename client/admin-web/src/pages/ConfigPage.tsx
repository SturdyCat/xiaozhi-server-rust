/**
 * 配置页：壳 + 能力总览 + 预设 + 已迁移的配置段表单。
 *
 * 分区清单唯一来源是 `configSections.ts`（对齐 Kuikly `ConfigSections.kt`）。
 * Provider 只在本页挂载，避免其它页面无端拉取配置。
 * W4/C1 已迁移 server / aiui / audio（playback 挂在 audio 页尾）；
 * W5/C2 已迁移 vad / asr / command / wakeup；
 * W6/C3 已迁移 llm / tts；W7/C4 已迁移 nlu；W8/C5 已迁移 soul / memory；
 * W9/C6 已迁移 echomuse / tuya / mcp（只读）；W10/C7 已迁移 ota（动作面板，0 配置字段，
 * 不消费 SaveBar 的 dirtyCount / saving / hot / revision 语义）。
 * 其余分区仍显示占位卡，文案里的「W4–W11」保持不变。
 */

import { useEffect, useState, type ReactElement } from "react";
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
import { AdvancedModeProvider, useAdvancedMode } from "@/context/AdvancedModeContext";
import { ConfigDraftProvider, useConfigDraft } from "@/context/ConfigDraftContext";
import { getConfigMeta, saveImpactOf } from "@/lib/configApi";
import { useApiQuery } from "@/lib/useApiQuery";
import { useConfigSchema } from "@/lib/useConfigSchema";
import { cn, formatDateTime } from "@/lib/utils";

/** 未迁移分区的占位卡。 */
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

function ConfigPageContent(): ReactElement {
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

  const [active, setActive] = useState<string>("overview");
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
    [meta?.backend, meta?.loaded_from].filter((item) => item !== null && item !== undefined).join(" · ") ||
    undefined;
  // 两条非致命提示合并展示：草稿层的「已保存但回读失败」优先（信息更新），
  // 其次是「配置文件来源信息读取失败」（不影响配置本身的加载与保存）。
  const warning =
    draftWarning ??
    (metaQuery.error === null
      ? null
      : `配置文件来源信息读取失败：${metaQuery.error}`);

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

  return (
    <div className="mx-auto w-full max-w-6xl space-y-6">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="space-y-1">
          <h2 className="text-2xl font-semibold tracking-tight">配置</h2>
          <p className="text-sm text-muted-foreground">
            服务端参数与热生效档位；保存按字段增量提交。
          </p>
        </div>
        <div className="flex flex-wrap items-start gap-4">
          <div className="flex items-center gap-2">
            <span className="text-xs text-muted-foreground">
              {advancedMode ? "高级" : "简单"}
            </span>
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
              const items = sectionsOf(group.id);
              if (items.length === 0) {
                return null;
              }
              return (
                <div key={group.id} className="space-y-1">
                  <p className="px-3 text-xs font-medium text-muted-foreground">
                    {group.label}
                  </p>
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
              className="flex flex-wrap items-start justify-between gap-3 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
            >
              <div className="space-y-1">
                <p>
                  配置已被其他窗口/端修改，本次保存已取消（避免覆盖别人的改动）。
                </p>
                <p className="text-xs text-destructive/80">
                  你当前的修改仍保留在页面上，可先复制再重新加载。
                </p>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={() => void resolveConflict()}
              >
                重新加载并放弃本地改动
              </Button>
            </div>
          ) : null}

          {active === "overview" ? (
            <ConfigOverviewSection onJump={setActive} />
          ) : active === "presets" ? (
            <ConfigPresetsSection />
          ) : active === "vad" ? (
            <VadSection />
          ) : active === "asr" ? (
            <AsrSection />
          ) : active === "nlu" ? (
            <NluSection />
          ) : active === "llm" ? (
            <LlmSection />
          ) : active === "tts" ? (
            <TtsSection />
          ) : active === "command" ? (
            <CommandSection />
          ) : active === "wakeup" ? (
            <WakeupSection />
          ) : active === "soul" ? (
            <SoulSection />
          ) : active === "memory" ? (
            <MemorySection />
          ) : active === "server" ? (
            <ServerSection />
          ) : active === "aiui" ? (
            <AIUISection />
          ) : active === "audio" ? (
            <>
              <AudioSection />
              <PlaybackSection />
            </>
          ) : active === "echomuse" ? (
            <EchomuseSection />
          ) : active === "tuya" ? (
            <TuyaSection />
          ) : active === "mcp" ? (
            <McpSection />
          ) : active === "ota" ? (
            // ota 是纯动作面板：不消费 SaveBar 的 dirtyCount / saving / hot / revision 语义。
            <OtaSection />
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

export function ConfigPage(): ReactElement {
  return (
    <AdvancedModeProvider>
      <ConfigDraftProvider>
        <ConfigPageContent />
      </ConfigDraftProvider>
    </AdvancedModeProvider>
  );
}
