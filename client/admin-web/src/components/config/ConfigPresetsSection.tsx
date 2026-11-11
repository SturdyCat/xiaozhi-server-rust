/**
 * 配置页「预设」（C0）。
 *
 * - 预设是**全局配置快照**（不含 per-device overlay）；
 * - 应用预设会覆盖当前配置，成功后必须调用 `useConfigDraft().reload()`
 *   重新取 baseline/revision，否则本地草稿与服务器脱节；
 * - 管理台统一走 POST 删除别名（server 侧同时保留 DELETE 路由）。
 */

import type { ReactElement } from "react";
import { useState } from "react";
import { ConfirmDialog } from "@/components/feedback/ConfirmDialog";
import { DataState } from "@/components/feedback/DataState";
import { SectionCard } from "@/components/config/SectionCard";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useToast } from "@/components/feedback/ToastHost";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import {
  applyPreset,
  configErrorMessage,
  deletePreset,
  listPresets,
  savePreset,
} from "@/lib/configApi";
import { useApiQuery } from "@/lib/useApiQuery";

export function ConfigPresetsSection(): ReactElement {
  const { show } = useToast();
  const { reload } = useConfigDraft();
  const { data, loading, error, refetch } = useApiQuery(listPresets);

  const presets = data ?? [];
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [pendingApply, setPendingApply] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);

  const trimmedName = name.trim();

  const handleCreate = async (): Promise<void> => {
    if (trimmedName === "" || saving) {
      return;
    }
    setSaving(true);
    try {
      const result = await savePreset(trimmedName);
      show(result.message?.trim() ? result.message : `已保存预设「${trimmedName}」`, "success");
      setName("");
      await refetch();
    } catch (err: unknown) {
      show(configErrorMessage(err), "error");
    } finally {
      setSaving(false);
    }
  };

  const handleApply = async (presetName: string): Promise<void> => {
    setPendingApply(null);
    setBusy(presetName);
    try {
      const result = await applyPreset(presetName);
      // 应用后服务端配置已变：必须重取 baseline/revision，且**要等它真的成功**
      // 再报「已应用」，否则界面还是旧配置却提示成功。
      const reloaded = await reload();
      if (!reloaded) {
        show(
          `预设「${presetName}」已应用到服务端，但重新读取配置失败：请手动点「重新加载」确认当前值。`,
          "error",
        );
        return;
      }
      show(result.message?.trim() ? result.message : `已应用预设「${presetName}」`, "success");
    } catch (err: unknown) {
      show(configErrorMessage(err), "error");
    } finally {
      setBusy(null);
    }
  };

  const handleDelete = async (presetName: string): Promise<void> => {
    setPendingDelete(null);
    setBusy(presetName);
    try {
      const result = await deletePreset(presetName);
      show(result.message?.trim() ? result.message : `已删除预设「${presetName}」`, "success");
      await refetch();
    } catch (err: unknown) {
      show(configErrorMessage(err), "error");
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="space-y-4">
      <SectionCard title="已保存预设" description="预设是全局配置快照，不含 per-device overlay。">
        <DataState
          loading={loading}
          error={error}
          empty={presets.length === 0}
          emptyText="暂无预设"
          onRetry={() => void refetch()}
        >
          <ul className="divide-y divide-border">
            {presets.map((preset) => (
              <li
                key={preset.name}
                className="flex flex-wrap items-center justify-between gap-2 py-2 first:pt-0 last:pb-0"
              >
                <span className="font-mono text-sm">{preset.name}</span>
                <span className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={busy !== null}
                    onClick={() => setPendingApply(preset.name)}
                  >
                    应用
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="text-destructive"
                    disabled={busy !== null}
                    onClick={() => setPendingDelete(preset.name)}
                  >
                    删除
                  </Button>
                </span>
              </li>
            ))}
          </ul>
        </DataState>
      </SectionCard>

      <SectionCard
        title="保存当前配置为预设"
        description="把当前生效配置保存为全局快照，后续可一键应用到其他环境。"
      >
        <form
          className="flex flex-wrap items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            void handleCreate();
          }}
        >
          <div className="min-w-[200px] flex-1 space-y-1.5">
            <Label htmlFor="preset-name" className="text-xs">
              预设名称
            </Label>
            <Input
              id="preset-name"
              value={name}
              placeholder="例如：低延迟调优"
              onChange={(event) => setName(event.target.value)}
            />
          </div>
          <Button type="submit" disabled={trimmedName === "" || saving}>
            {saving ? "保存中…" : "保存为预设"}
          </Button>
        </form>
      </SectionCard>

      <ConfirmDialog
        open={pendingApply !== null}
        title="应用预设"
        description={
          pendingApply === null
            ? undefined
            : `应用预设「${pendingApply}」会覆盖当前配置（含未保存的本地修改），不可撤销。`
        }
        confirmText="应用"
        onConfirm={() => {
          if (pendingApply !== null) {
            void handleApply(pendingApply);
          }
        }}
        onCancel={() => setPendingApply(null)}
      />

      <ConfirmDialog
        open={pendingDelete !== null}
        title="删除预设"
        description={
          pendingDelete === null ? undefined : `删除预设「${pendingDelete}」后不可恢复。`
        }
        confirmText="删除"
        destructive
        onConfirm={() => {
          if (pendingDelete !== null) {
            void handleDelete(pendingDelete);
          }
        }}
        onCancel={() => setPendingDelete(null)}
      />
    </div>
  );
}
