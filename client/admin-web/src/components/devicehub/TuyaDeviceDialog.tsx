/**
 * 涂鸦设备详情弹窗（W12 devicehub；TuyaDeviceDialog.kt:20-28 + TuyaDetailPage.kt:89-100）。
 *
 * - 三块：设备信息 / 本地别名 / DP 控制；
 * - DP 列表的 `writable`/`control` 由服务端下发（boolean→switch / number→number /
 *   其余 readonly），**客户端不重算**（研究 §12 QA 关注点铁律）；
 * - 控制失败三归类逐字（TuyaState.kt:465-471）：offline→「设备离线：…」/
 *   timeout→「设备无响应：…」/ unsupported→「协议不支持：…」，`advice` 中文原文括注；
 * - 本地别名仅本管理台展示，不影响云端名称；空串 = 清除；
 * - 点遮罩不关闭（Dialog onInteractOutside preventDefault）。
 */

import { useCallback, useEffect, useState, type ReactElement } from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { DataState } from "@/components/feedback/DataState";
import { useToast } from "@/components/feedback/ToastHost";
import {
  getTuyaDevice,
  saveTuyaAlias,
  setTuyaDp,
  TuyaApiError,
  type TuyaDeviceDetail,
  type TuyaDp,
} from "@/lib/tuyaApi";

export interface TuyaDeviceDialogProps {
  deviceId: string | null;
  onClose: () => void;
  /** 别名/DP 变化后由父级刷新设备目录。 */
  onChanged: () => void;
}

/** 控制失败三归类（TuyaState.kt:465-471 逐字 + advice 括注）。 */
function classifyFailure(error: unknown): string {
  if (error instanceof TuyaApiError) {
    const base =
      error.kind === "offline"
        ? `设备离线：${error.serverError}`
        : error.kind === "timeout"
          ? `设备无响应：${error.serverError}`
          : error.kind === "unsupported"
            ? `协议不支持：${error.serverError}`
            : error.serverError !== ""
              ? error.serverError
              : error.message;
    return error.advice !== null ? `${base}（${error.advice}）` : base;
  }
  return error instanceof Error ? error.message : String(error);
}

export function TuyaDeviceDialog({
  deviceId,
  onClose,
  onChanged,
}: TuyaDeviceDialogProps): ReactElement {
  const toast = useToast();
  const open = deviceId !== null;
  const [detail, setDetail] = useState<TuyaDeviceDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [aliasDraft, setAliasDraft] = useState("");
  const [aliasSaving, setAliasSaving] = useState(false);
  const [dpBusy, setDpBusy] = useState<string | null>(null);
  const [numberDrafts, setNumberDrafts] = useState<Record<string, string>>({});

  const load = useCallback(async (id: string): Promise<void> => {
    setLoading(true);
    setLoadError(null);
    try {
      const result = await getTuyaDevice(id);
      setDetail(result);
      setAliasDraft(result.name);
    } catch (error) {
      setDetail(null);
      setLoadError(error instanceof Error ? error.message : String(error));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (deviceId !== null) {
      void load(deviceId);
    } else {
      setDetail(null);
      setLoadError(null);
      setAliasDraft("");
      setNumberDrafts({});
    }
  }, [deviceId, load]);

  const handleSaveAlias = async (): Promise<void> => {
    if (deviceId === null) {
      return;
    }
    setAliasSaving(true);
    try {
      await saveTuyaAlias(deviceId, aliasDraft.trim());
      toast.show("别名已保存", "success");
      await load(deviceId);
      onChanged();
    } catch (error) {
      toast.show(`保存失败：${error instanceof Error ? error.message : String(error)}`, "error");
    } finally {
      setAliasSaving(false);
    }
  };

  const handleSetDp = async (dp: TuyaDp, value: boolean | number): Promise<void> => {
    if (deviceId === null) {
      return;
    }
    setDpBusy(dp.dp);
    try {
      await setTuyaDp(deviceId, dp.dp, value);
      toast.show(`已下发 DP ${dp.dp}`, "success");
      await load(deviceId);
      onChanged();
    } catch (error) {
      toast.show(classifyFailure(error), "error");
    } finally {
      setDpBusy(null);
    }
  };

  const renderDpRow = (dp: TuyaDp): ReactElement => {
    const busy = dpBusy === dp.dp;
    return (
      <div
        key={dp.dp}
        className="flex flex-wrap items-center justify-between gap-2 rounded-md border px-3 py-2"
      >
        <div className="min-w-0">
          <p className="text-sm font-medium">DP {dp.dp}</p>
          <p className="text-xs text-muted-foreground">
            当前：{dp.valueText !== "" ? dp.valueText : "—"} · {dp.control}
          </p>
        </div>
        {dp.control === "switch" ? (
          <div className="flex items-center gap-2">
            <Switch
              checked={dp.value === true}
              disabled={busy || !dp.writable}
              onCheckedChange={(checked) => void handleSetDp(dp, checked)}
            />
            <span className="text-xs text-muted-foreground">{dp.value === true ? "开" : "关"}</span>
          </div>
        ) : dp.control === "number" ? (
          <div className="flex items-center gap-2">
            <Input
              type="number"
              className="w-28"
              value={numberDrafts[dp.dp] ?? (typeof dp.value === "number" ? String(dp.value) : "")}
              disabled={busy || !dp.writable}
              onChange={(event) =>
                setNumberDrafts((previous) => ({ ...previous, [dp.dp]: event.target.value }))
              }
            />
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy || !dp.writable || (numberDrafts[dp.dp] ?? "") === ""}
              onClick={() => {
                const raw = numberDrafts[dp.dp] ?? "";
                const parsed = Number(raw);
                if (Number.isFinite(parsed)) {
                  void handleSetDp(dp, parsed);
                } else {
                  toast.show(`DP ${dp.dp} 需要数字值`, "error");
                }
              }}
            >
              {busy ? "下发中…" : "下发"}
            </Button>
          </div>
        ) : (
          <Badge variant="outline">只读</Badge>
        )}
      </div>
    );
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          onClose();
        }
      }}
    >
      <DialogContent
        className="max-h-[85vh] max-w-xl overflow-y-auto"
        onInteractOutside={(event) => {
          // 点遮罩不关闭（对齐旧端弹窗 dismissPolicy outsideClick=false）。
          event.preventDefault();
        }}
      >
        <DialogHeader>
          <DialogTitle>{detail?.name || deviceId || "涂鸦设备"}</DialogTitle>
        </DialogHeader>
        {deviceId !== null && loading && detail === null ? (
          <p className="text-sm text-muted-foreground">正在读取涂鸦设备详情…</p>
        ) : loadError !== null ? (
          <DataState
            loading={false}
            error={loadError}
            onRetry={() => {
              if (deviceId !== null) {
                void load(deviceId);
              }
            }}
          >
            <></>
          </DataState>
        ) : detail !== null ? (
          <div className="space-y-4">
            {/* 设备信息 */}
            <div className="space-y-1 rounded-lg border p-3">
              <div className="flex items-center gap-2">
                <Badge variant="secondary">涂鸦</Badge>
                <Badge variant="outline">{detail.online ? "在线" : "离线"}</Badge>
              </div>
              <p className="text-xs text-muted-foreground">
                型号：{detail.model !== "" ? detail.model : "型号未知"}
              </p>
              <p className="text-xs text-muted-foreground">
                IP：{detail.ip !== "" ? detail.ip : "IP 未上报"}
              </p>
              <p className="text-xs text-muted-foreground">设备 ID：{detail.deviceId}</p>
            </div>

            {/* 本地别名（TuyaDetailPage.kt:89-100 逐字） */}
            <div className="space-y-2 rounded-lg border p-3">
              <p className="text-sm font-medium">本地别名（仅本管理台展示，不影响云端名称）</p>
              <div className="flex items-center gap-2">
                <Input
                  value={aliasDraft}
                  placeholder="留空 = 使用设备名"
                  disabled={aliasSaving}
                  onChange={(event) => setAliasDraft(event.target.value)}
                />
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={aliasSaving}
                  onClick={() => void handleSaveAlias()}
                >
                  保存别名
                </Button>
              </div>
            </div>

            {/* DP 控制（control 由服务端下发，客户端不重算） */}
            <div className="space-y-2">
              <p className="text-sm font-medium">DP 控制</p>
              {detail.dps.length === 0 ? (
                <p className="text-xs text-muted-foreground">该设备未上报任何数据点。</p>
              ) : (
                detail.dps.map(renderDpRow)
              )}
            </div>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
