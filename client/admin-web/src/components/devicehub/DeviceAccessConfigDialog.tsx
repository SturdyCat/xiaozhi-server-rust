/**
 * 「设备接入配置」齿轮弹层（W12 devicehub；DeviceManagementCenter.kt:283-437 逐字移植）。
 *
 * 一次性配置口径：接入开关与凭据只在这里和配置页维护；设备列表用顶部刷新按钮更新。
 * - EchoMuse 只放状态与「打开接入配置」跳转（开关/令牌/欢迎语在 Config C6 的
 *   echomuse 节点，唯一配置表单不复制第二份）+ `/em/status` 设备面只读补充；
 * - Tuya 凭证表单就在这里（POST /api/tuya/credentials 是唯一录入路径）+ 立即同步 +
 *   「Tuya 接入」配置跳转（开关/同步周期在全局配置）；
 * - 弹层点遮罩不关闭（Dialog onInteractOutside preventDefault）；
 * - **不接 ConfigDraftContext / SaveBar**（0 配置字段纯运行面板，同 W10 形态）。
 */

import { useEffect, useState, type ReactElement } from "react";
import { Link } from "react-router-dom";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { useToast } from "@/components/feedback/ToastHost";
import { TuyaCredentialsCard } from "@/components/devicehub/TuyaCredentialsCard";
import { getEmStatus, type EmStatus } from "@/lib/echomuseApi";
import { syncTuyaNow, tuyaAgeText, tuyaResultLabel, type TuyaStatus } from "@/lib/tuyaApi";

export interface DeviceAccessConfigDialogProps {
  open: boolean;
  onClose: () => void;
  /** EchoMuse 接入状态（/api/echomuse/devices 的 enabled；null = 未加载）。 */
  echoEnabled: boolean | null;
  /** 涂鸦状态（status 卡与凭证卡展示用）；null = 未加载或未编入。 */
  tuyaStatus: TuyaStatus | null;
  /** 凭证/同步变化后由父级刷新 status 与设备目录。 */
  onTuyaChanged: () => void;
}

export function DeviceAccessConfigDialog({
  open,
  onClose,
  echoEnabled,
  tuyaStatus,
  onTuyaChanged,
}: DeviceAccessConfigDialogProps): ReactElement {
  const toast = useToast();
  const [syncing, setSyncing] = useState(false);
  const [emStatus, setEmStatus] = useState<EmStatus | null>(null);
  const tuyaEnabled = tuyaStatus?.enabled === true;

  // 打开时读一次 /em/status（G0 无鉴权只读）：设备面 enabled + 会话中设备数。
  useEffect(() => {
    if (!open) {
      setEmStatus(null);
      return;
    }
    let cancelled = false;
    getEmStatus()
      .then((status) => {
        if (!cancelled) {
          setEmStatus(status);
        }
      })
      .catch(() => {
        // 读取失败静默（本行是补充信息，不阻塞配置动作；诚实口径是不展示而非编造）。
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  const handleSync = async (): Promise<void> => {
    setSyncing(true);
    try {
      const result = await syncTuyaNow();
      // TuyaState.kt:250-270：skipped = 60s 闸门/并发拦下，不算失败也不报成功
      // （并发场景服务端同样回 skipped + message「已有一次同步在进行中」，无 "busy" 值）。
      if (result.result === "skipped") {
        toast.show(`已跳过：${result.message || tuyaResultLabel(result.result)}`, "info");
      } else if (result.result === "cloud_auth_failed" || result.result === "network_error") {
        toast.show(`${tuyaResultLabel(result.result)}：${result.message}`, "error");
      } else {
        toast.show(
          `同步${tuyaResultLabel(result.result)}${result.message ? `：${result.message}` : ""}`,
          result.result === "ok" ? "success" : "info",
        );
      }
      onTuyaChanged();
    } catch (error) {
      toast.show(error instanceof Error ? error.message : String(error), "error");
    } finally {
      setSyncing(false);
    }
  };

  const lastSyncLine = ((): string => {
    const last = tuyaStatus?.lastSync ?? null;
    if (last === null) {
      return "从未同步";
    }
    return `最近同步：${tuyaResultLabel(last.result)} · ${last.deviceCount} 台 · ${tuyaAgeText(last.ageMs)}`;
  })();

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
          // 点遮罩不关闭（对齐旧端 dismissOnOutside=false）；ESC 仍可关闭。
          event.preventDefault();
        }}
      >
        <DialogHeader>
          <DialogTitle>设备接入配置</DialogTitle>
          <DialogDescription>
            一次性配置：接入开关与凭据只在这里和配置页维护；设备列表用顶部刷新按钮更新。
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {/* EchoMuse 块（DeviceManagementCenter.kt:373-390 逐字） */}
          <div className="space-y-2 rounded-lg border p-3">
            <div className="flex items-center justify-between gap-2">
              <p className="text-sm font-medium">EchoMuse 接入</p>
              <p className="text-xs text-muted-foreground">
                {echoEnabled === null ? "状态未知" : echoEnabled ? "已启用" : "未启用"}
              </p>
            </div>
            <p className="text-xs text-muted-foreground">
              开关 / 接入令牌 / 欢迎语在「语音流程 → 终端与音频入口 → EchoMuse
              接入」节点编辑（重启生效）。
            </p>
            {emStatus !== null ? (
              <p className="text-xs text-muted-foreground">
                /em/ 设备面：{emStatus.enabled ? "已启用" : "未启用"}
                {emStatus.enabled ? ` · ${emStatus.devices.length} 台在会话中` : ""}
              </p>
            ) : null}
            <Button type="button" variant="outline" size="sm" asChild onClick={onClose}>
              <Link to="/config">打开接入配置</Link>
            </Button>
            <p className="text-xs text-muted-foreground">
              配置页无深链参数，打开后请选「EchoMuse」段。
            </p>
          </div>

          {/* Tuya 块（DeviceManagementCenter.kt:392-437 逐字） */}
          <div className="space-y-2 rounded-lg border p-3">
            <div className="flex items-center justify-between gap-2">
              <p className="text-sm font-medium">涂鸦设备</p>
              <p className="text-xs text-muted-foreground">
                {!tuyaEnabled
                  ? "未启用"
                  : tuyaStatus?.configured === true
                    ? `已配置 · ${tuyaStatus.accessIdMasked} · ${tuyaStatus.region}`
                    : "凭证未配置"}
              </p>
            </div>
            {!tuyaEnabled ? (
              <p className="text-xs text-destructive">
                涂鸦设备未启用：/api/tuya/* 全部 404（凭证录入同样不可用）。先到「Tuya
                接入」开启（保存即生效），再回来录入凭证。
              </p>
            ) : null}
            {tuyaEnabled ? (
              <TuyaCredentialsCard status={tuyaStatus} disabled={false} onChanged={onTuyaChanged} />
            ) : null}
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={!tuyaEnabled || syncing}
                onClick={() => void handleSync()}
              >
                {syncing ? "同步中…" : "立即同步"}
              </Button>
              <Button type="button" variant="outline" size="sm" asChild onClick={onClose}>
                <Link to="/config">打开接入配置</Link>
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">{lastSyncLine}</p>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
