/**
 * EchoMuse 扩展工具工作区（W12 devicehub；R11 拍板只做 3 项：models / diagnostics /
 * compat。music（DLNA）与 ble（GATT 工具）本轮不做——不出现在 UI，端点不封装）。
 *
 * 对齐旧端 EchoMuseExtensionContent.kt / EchoMuseOfficialEntry.kt：
 * - models「模型文件」：复用 W2 Files 页能力（GET /api/files?path=oww-models + 上传
 *   通道），**上传 ≠ 安装**（AGENTS.md:118）；oww-models 清单刻意安静的历史口径在
 *   web 侧收敛为「失败就地显示，不落到空目录」（AGENTS.md:159 Files 教训）；
 *   OWW 资产五态只读展示（读不到设备一律 unknown，绝不假填）；install-vad 不暴露
 *   （官方页 /em/ 承担资产安装）；
 * - diagnostics「原始诊断」：runtime + system/info + 复制 + 重启二次确认
 *   （EchoMuseExtensionContent.kt:40-67 逐字，含「请求发送不代表重启已完成」诚实锚点）；
 * - compat「设备兼容」：设备上报 settings 的只读兼容视角（编辑由 /em/ 官方页
 *   config 页签承载，本波不做 40 键编辑器）；
 * - 弹窗点遮罩不关闭；busy 门禁：全局 busy 非空禁用所有按钮。
 */

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type ReactElement,
} from "react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Badge } from "@/components/ui/badge";
import { useToast } from "@/components/feedback/ToastHost";
import { copyText } from "@/lib/clipboard";
import { getFileList, uploadFile, type FileEntry } from "@/lib/adminApi";
import {
  getEchomuseDevice,
  getEchomuseRuntime,
  getEchomuseSystemInfo,
  getOwwState,
  rebootEchomuseDevice,
  type EchomuseDevice,
  type EchomuseRuntime,
  type EchomuseShellOutput,
  type OwwState,
} from "@/lib/echomuseApi";

export interface ExtensionWorkspaceProps {
  device: EchomuseDevice;
  open: boolean;
  onClose: () => void;
}

type ExtensionTab = "models" | "diagnostics" | "compat";

const EXTENSION_TABS: { id: ExtensionTab; label: string }[] = [
  { id: "models", label: "模型文件" },
  { id: "diagnostics", label: "原始诊断" },
  { id: "compat", label: "设备兼容" },
];

/** 把字节数格式化为 B / KB / MB（Files 页同口径）。 */
function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "—";
  }
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${value} ${units[unit]}` : `${value.toFixed(1)} ${units[unit]}`;
}

// ============================================================
// models 页签
// ============================================================

function ModelsTab({ deviceId, busy }: { deviceId: string; busy: boolean }): ReactElement {
  const toast = useToast();
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [entries, setEntries] = useState<FileEntry[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [listLoading, setListLoading] = useState(true);
  const [uploading, setUploading] = useState(false);
  const [oww, setOww] = useState<OwwState | null>(null);
  const [owwError, setOwwError] = useState<string | null>(null);

  const refreshList = useCallback(async (): Promise<void> => {
    setListLoading(true);
    setListError(null);
    try {
      const listing = await getFileList("oww-models");
      setEntries(listing.entries.filter((entry) => !entry.dir));
    } catch (error) {
      // 列表请求失败不得落到「该目录为空」（AGENTS.md:159 Files 教训）。
      setEntries(null);
      setListError(error instanceof Error ? error.message : String(error));
    } finally {
      setListLoading(false);
    }
  }, []);

  // OWW 资产五态只读；读不到设备一律 unknown（服务端保证），读取失败就地显示。
  const refreshOww = useCallback(async (): Promise<void> => {
    setOwwError(null);
    try {
      setOww(await getOwwState(deviceId));
    } catch (error) {
      setOww(null);
      setOwwError(error instanceof Error ? error.message : String(error));
    }
  }, [deviceId]);

  useEffect(() => {
    void refreshList();
    void refreshOww();
  }, [refreshList, refreshOww]);

  const handleUpload = async (event: ChangeEvent<HTMLInputElement>): Promise<void> => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) {
      return;
    }
    setUploading(true);
    try {
      await uploadFile(`oww-models/${file.name}`, file, { overwrite: true });
      toast.show(`已上传 ${file.name}（上传不等于安装）`, "success");
      await refreshList();
    } catch (error) {
      toast.show(`上传失败：${error instanceof Error ? error.message : String(error)}`, "error");
    } finally {
      setUploading(false);
    }
  };

  return (
    <div className="space-y-3">
      <p className="text-xs text-muted-foreground">
        自定义唤醒词模型目录（oww-models/）。上传不等于安装：模型文件上传后，还需设备侧配置启用
        （owwModel / owwOnDevice，见 /em/ 官方页「配置」页签）。
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <input
          ref={fileInputRef}
          type="file"
          accept=".onnx"
          className="hidden"
          disabled={busy || uploading}
          onChange={(event) => void handleUpload(event)}
        />
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={busy || uploading}
          onClick={() => fileInputRef.current?.click()}
        >
          {uploading ? "上传中…" : "上传模型文件"}
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={busy}
          onClick={() => void refreshList()}
        >
          刷新列表
        </Button>
      </div>
      {listLoading && entries === null ? (
        <p className="text-xs text-muted-foreground">正在读取 oww-models/ 清单…</p>
      ) : listError !== null ? (
        <p className="text-sm text-destructive">oww-models/ 清单读取失败：{listError}</p>
      ) : entries !== null && entries.length === 0 ? (
        <p className="text-xs text-muted-foreground">oww-models/ 目录为空。</p>
      ) : entries !== null ? (
        <div className="space-y-1">
          {entries.map((entry) => (
            <div
              key={entry.name}
              className="flex items-center justify-between gap-2 rounded-md border px-3 py-1.5"
            >
              <span className="truncate text-sm">{entry.name}</span>
              <span className="shrink-0 text-xs text-muted-foreground">
                {formatBytes(entry.size)}
                {entry.name.endsWith(".onnx") ? " · .onnx" : ""}
              </span>
            </div>
          ))}
        </div>
      ) : null}
      <div className="rounded-md border px-3 py-2">
        {owwError !== null ? (
          <p className="text-xs text-destructive">OWW 资产状态读取失败：{owwError}</p>
        ) : oww === null ? (
          <p className="text-xs text-muted-foreground">正在读取 OWW 资产状态…</p>
        ) : (
          <p className="text-xs text-muted-foreground">
            OWW 资产（只读）：VAD {oww.vad.state} · ORT {oww.ort.state} · 可用空间{" "}
            {oww.freeMb !== null ? `${oww.freeMb} MB` : "—"}
          </p>
        )}
      </div>
    </div>
  );
}

// ============================================================
// diagnostics 页签
// ============================================================

function DiagnosticsTab({ deviceId, busy }: { deviceId: string; busy: boolean }): ReactElement {
  const toast = useToast();
  const [runtime, setRuntime] = useState<EchomuseRuntime | null>(null);
  const [runtimeError, setRuntimeError] = useState<string | null>(null);
  const [info, setInfo] = useState<EchomuseShellOutput | null>(null);
  const [infoError, setInfoError] = useState<string | null>(null);
  const [rebootConfirmOpen, setRebootConfirmOpen] = useState(false);

  const readRuntime = async (): Promise<void> => {
    setRuntimeError(null);
    try {
      setRuntime(await getEchomuseRuntime(deviceId));
    } catch (error) {
      setRuntime(null);
      setRuntimeError(error instanceof Error ? error.message : String(error));
    }
  };

  const readInfo = async (): Promise<void> => {
    setInfoError(null);
    try {
      setInfo(await getEchomuseSystemInfo(deviceId));
    } catch (error) {
      setInfo(null);
      setInfoError(error instanceof Error ? error.message : String(error));
    }
  };

  const handleCopy = async (): Promise<void> => {
    const payload = {
      device_id: deviceId,
      runtime: runtime?.raw ?? null,
      system_info: info !== null ? { output: info.output, exit_code: info.exitCode } : null,
    };
    const copied = await copyText(JSON.stringify(payload, null, 2));
    // EchoMuseExtensionContent.kt:52-56 逐字。
    toast.show(copied ? "诊断内容已复制" : "复制失败：剪贴板不可用", copied ? "success" : "error");
  };

  const handleReboot = async (): Promise<void> => {
    setRebootConfirmOpen(false);
    try {
      const result = await rebootEchomuseDevice(deviceId);
      // 诚实化锚点 9：202 {dispatched:true, rebooted:false}——确认调度 ≠ 已完成。
      toast.show(
        result.rebooted
          ? "重启请求已发送，设备已确认重启。"
          : "重启请求已发送；调度 ≠ 已完成，执行结果请在设备上核对。",
        "success",
      );
      void readRuntime();
    } catch (error) {
      toast.show(error instanceof Error ? error.message : String(error), "error");
    }
  };

  return (
    <div className="space-y-3">
      {/* 当前会话最近报告 */}
      <div className="space-y-2 rounded-lg border p-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-sm font-medium">当前会话最近报告</p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => void readRuntime()}
          >
            读取设备报告
          </Button>
        </div>
        {runtimeError !== null ? (
          <p className="text-sm text-destructive">{runtimeError}</p>
        ) : runtime === null ? (
          <p className="text-xs text-muted-foreground">
            尚未读取；设备离线时回包为全 null 快照（不是错误）。
          </p>
        ) : (
          <div className="space-y-1">
            <p className="text-xs text-muted-foreground">
              online：{runtime.online ? "是" : "否"} · session：{runtime.sessionId ?? "—"} ·
              updated： {runtime.updatedAt ?? "—"}
              {runtime.link !== null
                ? ` · link：authenticated=${runtime.link.authenticated ? "是" : "否"}/token_required=${runtime.link.tokenRequired ? "是" : "否"}`
                : ""}
            </p>
            <pre className="max-h-48 overflow-auto rounded-md bg-muted/50 p-2 text-xs">
              {JSON.stringify(runtime.raw, null, 2)}
            </pre>
          </div>
        )}
      </div>

      {/* 原始系统信息 */}
      <div className="space-y-2 rounded-lg border p-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-sm font-medium">原始系统信息</p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => void readInfo()}
          >
            读取原始系统信息
          </Button>
        </div>
        {infoError !== null ? (
          <p className="text-sm text-destructive">{infoError}</p>
        ) : info === null ? (
          <p className="text-xs text-muted-foreground">
            尚未读取（固定 INFO 命令，客户端不插值 shell）。
          </p>
        ) : (
          <pre className="max-h-48 overflow-auto rounded-md bg-muted/50 p-2 text-xs">
            {info.output}
          </pre>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={busy || (runtime === null && info === null)}
          onClick={() => void handleCopy()}
        >
          复制诊断内容
        </Button>
        <Button
          type="button"
          variant="destructive"
          size="sm"
          disabled={busy}
          onClick={() => setRebootConfirmOpen(true)}
        >
          重启设备
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        自定义诊断包含原始设备与网络信息，请仅向可信人员分享。
      </p>

      {/* 重启二次确认（EchoMuseExtensionContent.kt:58-64 逐字；点遮罩不关闭） */}
      <AlertDialog
        open={rebootConfirmOpen}
        onOpenChange={() => {
          // 忽略 ESC / 遮罩关闭请求：只能走「发送 / 取消」两个按钮。
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>确认重启设备？</AlertDialogTitle>
            <AlertDialogDescription>
              当前连接与语音会中断。请求发送不代表重启已完成。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel
              onClick={() => {
                setRebootConfirmOpen(false);
              }}
            >
              取消
            </AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => void handleReboot()}>
              发送重启请求
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

// ============================================================
// compat 页签
// ============================================================

function CompatTab({ deviceId, busy }: { deviceId: string; busy: boolean }): ReactElement {
  const [settings, setSettings] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async (): Promise<void> => {
    setLoading(true);
    setError(null);
    try {
      const detail = await getEchomuseDevice(deviceId);
      setSettings(detail.settings);
    } catch (caught) {
      setSettings(null);
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setLoading(false);
    }
  }, [deviceId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <div className="space-y-3">
      <p className="text-xs text-muted-foreground">
        设备兼容视角：设备上报 settings 的只读快照。编辑请用 /em/ 官方页「配置」页签 （本管理台不做
        40 键编辑器；配置保存走官方页的增量下发口径）。
      </p>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        disabled={busy}
        onClick={() => void refresh()}
      >
        重新读取
      </Button>
      {loading && settings === null ? (
        <p className="text-xs text-muted-foreground">正在读取设备设置快照…</p>
      ) : error !== null ? (
        <p className="text-sm text-destructive">{error}</p>
      ) : settings === null || Object.keys(settings).length === 0 ? (
        <p className="text-xs text-muted-foreground">设备未上报任何 settings 键。</p>
      ) : (
        <div className="space-y-1">
          {Object.entries(settings).map(([key, value]) => (
            <div
              key={key}
              className="flex items-start justify-between gap-3 rounded-md border px-3 py-1.5"
            >
              <span className="shrink-0 font-mono text-xs">{key}</span>
              <span className="min-w-0 break-all text-right font-mono text-xs text-muted-foreground">
                {typeof value === "string" ? value : JSON.stringify(value)}
              </span>
            </div>
          ))}
        </div>
      )}
      {settings !== null ? (
        <p className="text-xs text-muted-foreground">
          共 {Object.keys(settings).length} 键。白名单外键服务端在保存时会丢弃且界面无提示
          （em/README.md 十项已知显示偏差之一，前端不编造绕过方式）。
        </p>
      ) : null}
    </div>
  );
}

// ============================================================
// 工作区弹窗
// ============================================================

export function ExtensionWorkspace({
  device,
  open,
  onClose,
}: ExtensionWorkspaceProps): ReactElement {
  const [tab, setTab] = useState<ExtensionTab>("models");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) {
      setTab("models");
      setBusy(false);
    }
  }, [open]);

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
        className="max-h-[90vh] max-w-3xl overflow-y-auto"
        onInteractOutside={(event) => {
          // 点遮罩不关闭（扩展模式 Tab outline 用 UNDERLINE 口径，交互对齐旧端）。
          event.preventDefault();
        }}
      >
        <DialogHeader>
          <DialogTitle className="flex flex-wrap items-center gap-2">
            自定义扩展 · {device.label || device.deviceId}
            <Badge variant="outline">{device.online ? "在线" : "离线"}</Badge>
          </DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="flex gap-4 border-b">
            {EXTENSION_TABS.map((item) => (
              <button
                key={item.id}
                type="button"
                className={
                  tab === item.id
                    ? "-mb-px border-b-2 border-primary pb-2 text-sm font-medium text-foreground"
                    : "-mb-px border-b-2 border-transparent pb-2 text-sm text-muted-foreground hover:text-foreground"
                }
                onClick={() => setTab(item.id)}
              >
                {item.label}
              </button>
            ))}
          </div>
          {tab === "models" ? <ModelsTab deviceId={device.deviceId} busy={busy} /> : null}
          {tab === "diagnostics" ? <DiagnosticsTab deviceId={device.deviceId} busy={busy} /> : null}
          {tab === "compat" ? <CompatTab deviceId={device.deviceId} busy={busy} /> : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
