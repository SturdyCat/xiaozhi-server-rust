/**
 * 设备中心页（W12 / devicehub；DeviceManagementCenter.kt 逐字移植 + 拍板裁剪）。
 *
 * 形态：顶部 title 栏（摘要 + 扩展工具切换 + 刷新 + 齿轮「设备接入配置」）→
 * EchoMuse / 涂鸦两分区卡 → 点卡片弹各自详情（EchoMuse = /em/ iframe 嵌入官方页；
 * 涂鸦 = TuyaDeviceDialog DP 弹窗）。扩展模式只留 EchoMuse 卡。
 *
 * ## 拍板裁剪（R4 / nav / R11）
 *
 * - R4：/em/ CDN 直连不 vendor——iframe 外方常驻诚实文案（emEmbed.ts）；
 * - nav：devicehub 是第 10 个静态导航项，独立页；与 Config C6 的 EchomuseSection /
 *   TuyaSection（应用级配置）职责分离，不合并；
 * - R11：扩展工具只做 models / diagnostics / compat 三项，music / ble 不做。
 *
 * ## 门控（严禁照搬 Kotlin device_plugins 注册门控）
 *
 * W11 已证伪 `device_plugins[]` 在服务端零匹配不下发。本页按**字段级 fail-closed**：
 * - EchoMuse 分区按 `GET /api/echomuse/devices` 的 `enabled`（404 disabled /
 *   HTML 200 归一为 UnavailableError.kind）；
 * - 涂鸦分区按 `GET /api/tuya/status` 的 `enabled` / `configured`。
 *
 * ## 0 配置字段纯运行面板（同 W10 OTA 形态）
 *
 * 不进 configSections.ts、不接 ConfigDraftContext / setField / SaveBar；凭证唯一
 * 录入路径 POST /api/tuya/credentials（齿轮弹层），不接 PUT 主配置链。
 *
 * ## 实时更新口径
 *
 * 服务端有 `echomuse_devices_changed` 推送（GET /api/admin/ws），本波采用研究允许的
 * 保守口径：**手动刷新按钮（W9 MCP 形态），无轮询、无长挂**——admin-web 尚无管理台
 * WS 订阅通道（getWebSocketUrl 零调用点），不自行发明帧协议。
 */

import { useCallback, useState, type ReactElement } from "react";
import { Link } from "react-router-dom";
import { Plus, RefreshCw, Settings2, Wrench } from "lucide-react";
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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { UnavailableNotice } from "@/components/feedback/UnavailableNotice";
import { useToast } from "@/components/feedback/ToastHost";
import { DeviceAccessConfigDialog } from "@/components/devicehub/DeviceAccessConfigDialog";
import { EchoMuseDetailDialog } from "@/components/devicehub/EchoMuseDetailDialog";
import { ExtensionWorkspace } from "@/components/devicehub/ExtensionWorkspace";
import { TuyaDeviceDialog } from "@/components/devicehub/TuyaDeviceDialog";
import { useApiQuery } from "@/lib/useApiQuery";
import {
  approveEchomuseDevice,
  deleteEchomuseDevice,
  getEchomuseDevices,
  EchomuseApiError,
  type EchomuseDevice,
} from "@/lib/echomuseApi";
import {
  getTuyaDevices,
  getTuyaStatus,
  tuyaAgeText,
  tuyaResultLabel,
  type TuyaDeviceRow,
  type TuyaStatus,
} from "@/lib/tuyaApi";

// ============================================================
// 分区提示（DeviceManagementCenter.kt 逐字）
// ============================================================

function SectionHint(props: { line: string; action?: ReactElement; danger?: boolean }): ReactElement {
  return (
    <div className="space-y-2 rounded-lg border border-dashed p-4">
      <p className={props.danger === true ? "text-sm text-destructive" : "text-sm"}>
        {props.line}
      </p>
      {props.action ?? null}
    </div>
  );
}

// ============================================================
// EchoMuse 设备卡
// ============================================================

function EchomuseCard(props: {
  device: EchomuseDevice;
  extensions: boolean;
  busy: boolean;
  onOpen: () => void;
  onApprove: () => void;
  onDelete: () => void;
}): ReactElement {
  const { device } = props;
  return (
    <Card>
      <CardHeader className="flex-row items-start justify-between gap-3 space-y-0">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <CardTitle className="text-base">{device.label || device.deviceId}</CardTitle>
            <Badge variant="secondary">{device.online ? "在线" : "离线"}</Badge>
            {!device.approved ? <Badge variant="outline">待批准</Badge> : null}
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-2">
        <p className="text-xs text-muted-foreground">
          {device.deviceId}
          {device.version !== "" ? ` · v${device.version}` : ""}
          {device.board !== "" ? ` · ${device.board}` : ""}
          {device.baseOs !== "" ? ` · ${device.baseOs}` : ""}
        </p>
        <p className="text-xs text-muted-foreground">
          IP：{device.ip !== "" ? device.ip : "IP 未上报"}
          {device.online && device.connectedSecs > 0
            ? ` · 本次连接 ${Math.floor(device.connectedSecs / 60)} 分钟`
            : ""}
        </p>
        <div className="flex flex-wrap gap-2">
          {props.extensions ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={props.busy}
              onClick={props.onOpen}
            >
              打开扩展工具
            </Button>
          ) : (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={props.busy}
              onClick={props.onOpen}
            >
              打开管理页
            </Button>
          )}
          {!device.approved ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={props.busy}
              onClick={props.onApprove}
            >
              批准
            </Button>
          ) : null}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={props.busy}
            onClick={props.onDelete}
          >
            删除记录
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

// ============================================================
// 涂鸦设备卡（TuyaPage.kt:125-176 口径）
// ============================================================

function TuyaCard(props: {
  device: TuyaDeviceRow;
  busy: boolean;
  onOpen: () => void;
}): ReactElement {
  const { device } = props;
  return (
    <Card>
      <CardHeader className="flex-row items-start justify-between gap-3 space-y-0">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <CardTitle className="text-base">{device.name !== "" ? device.name : device.deviceId}</CardTitle>
            <Badge variant="secondary">涂鸦</Badge>
            <Badge variant="outline">{device.online ? "在线" : "离线"}</Badge>
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-2">
        <p className="text-xs text-muted-foreground">
          {device.deviceId} · {device.model !== "" ? device.model : "型号未知"}
        </p>
        <p className="text-xs text-muted-foreground">IP：{device.ip !== "" ? device.ip : "IP 未上报"}</p>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={props.busy}
          onClick={props.onOpen}
        >
          设备详情
        </Button>
      </CardContent>
    </Card>
  );
}

// ============================================================
// 页面
// ============================================================

export function DevicehubPage(): ReactElement {
  const toast = useToast();
  const echoQ = useApiQuery(getEchomuseDevices, []);
  const tuyaStatusQ = useApiQuery(getTuyaStatus, []);
  const tuyaStatus: TuyaStatus | null = tuyaStatusQ.data;
  const tuyaEnabled = tuyaStatus?.enabled === true;

  // 涂鸦目录只在 enabled 时拉取（未启用时全部 404，不制造噪音）。
  const tuyaDevicesQ = useApiQuery(
    useCallback(async () => {
      if (!tuyaEnabled) {
        return { devices: [] as TuyaDeviceRow[], total: 0, onlineCount: 0 };
      }
      return getTuyaDevices();
    }, [tuyaEnabled]),
    [tuyaEnabled],
  );

  const [busy, setBusy] = useState<string | null>(null);
  const [extensions, setExtensions] = useState(false);
  const [gearOpen, setGearOpen] = useState(false);
  const [detailEmId, setDetailEmId] = useState<string | null>(null);
  const [extensionEmId, setExtensionEmId] = useState<string | null>(null);
  const [detailTuyaId, setDetailTuyaId] = useState<string | null>(null);
  const [deleteConfirmId, setDeleteConfirmId] = useState<string | null>(null);

  const refreshAll = useCallback((): void => {
    void echoQ.refetch();
    void tuyaStatusQ.refetch();
    void tuyaDevicesQ.refetch();
  }, [echoQ, tuyaStatusQ, tuyaDevicesQ]);

  const runAction = async (key: string, action: () => Promise<void>): Promise<void> => {
    setBusy(key);
    try {
      await action();
    } finally {
      setBusy(null);
    }
  };

  const echoData = echoQ.data;
  const echoEnabled = echoData?.enabled === true;
  const echoLoading = echoQ.loading && echoData === null;
  const tuyaDevices = tuyaDevicesQ.data?.devices ?? [];

  // 摘要（deviceHubSummary 口径：涂鸦未配置/未启用时如实说，不冒充 0 台）。
  const summary = ((): string => {
    const echoLoadingNow = echoLoading;
    const tuyaLoadingNow = (tuyaStatusQ.loading && tuyaStatus === null) || (tuyaEnabled && tuyaDevicesQ.loading && tuyaDevicesQ.data === null);
    if (echoLoadingNow && tuyaLoadingNow) {
      return "加载中…";
    }
    const parts: string[] = [];
    if (echoEnabled) {
      parts.push(`EchoMuse ${echoData?.devices.length ?? 0} 台`);
    } else {
      parts.push(echoQ.error === null && echoData !== null ? "EchoMuse 未启用" : "EchoMuse 不可用");
    }
    if (!tuyaEnabled) {
      parts.push(tuyaStatus !== null ? "涂鸦未启用" : "涂鸦不可用");
    } else {
      parts.push(`涂鸦 ${tuyaDevices.length} 台`);
    }
    const onlineCount =
      (echoData?.devices.filter((device) => device.online).length ?? 0) +
      tuyaDevices.filter((device) => device.online).length;
    parts.push(`在线 ${onlineCount}`);
    return parts.join(" · ");
  })();

  const handleApprove = (device: EchomuseDevice): void => {
    void runAction(`approve:${device.deviceId}`, async () => {
      try {
        const approved = await approveEchomuseDevice(device.deviceId);
        // EchoMuseState.kt:232 逐字。
        toast.show(`已批准 ${approved.label || approved.deviceId}`, "success");
        void echoQ.refetch();
      } catch (error) {
        if (error instanceof EchomuseApiError && error.status === 404) {
          toast.show("设备不存在（列表可能已过期，请刷新后重试）", "error");
        } else {
          toast.show(error instanceof Error ? error.message : String(error), "error");
        }
      }
    });
  };

  const handleDeleteEm = (device: EchomuseDevice): void => {
    // §13 实施清单：删除走 destructive 二次确认 + 点遮罩不关闭；确认后执行。
    setDeleteConfirmId(device.deviceId);
  };

  const deleteTarget =
    deleteConfirmId !== null
      ? (echoData?.devices.find((device) => device.deviceId === deleteConfirmId) ?? null)
      : null;

  const detailEm =
    detailEmId !== null
      ? (echoData?.devices.find((device) => device.deviceId === detailEmId) ?? null)
      : null;
  const extensionEm =
    extensionEmId !== null
      ? (echoData?.devices.find((device) => device.deviceId === extensionEmId) ?? null)
      : null;

  const echoError = echoQ.error;

  return (
    <div className="space-y-4">
      {/* title 栏（DeviceManagementCenter.kt:97-125 逐字口径） */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <h2 className="text-lg font-semibold">设备管理</h2>
          <p className="text-xs text-muted-foreground">
            {extensions ? "自定义扩展 · 选择设备" : summary}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy !== null}
            onClick={() => {
              setExtensions((previous) => !previous);
              setDetailEmId(null);
              setExtensionEmId(null);
              setDetailTuyaId(null);
            }}
          >
            <Wrench aria-hidden="true" className="size-4" />
            {extensions ? "返回设备管理" : "扩展工具"}
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={busy !== null}
            onClick={refreshAll}
          >
            <RefreshCw aria-hidden="true" className="size-4" />
            刷新
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            aria-label="设备接入配置"
            disabled={busy !== null}
            onClick={() => setGearOpen(true)}
          >
            <Settings2 aria-hidden="true" className="size-4" />
            设备接入配置
          </Button>
        </div>
      </div>

      {/* 两分区（扩展模式隐藏涂鸦区，只留 EchoMuse 卡） */}
      <div className={extensions ? "space-y-4" : "grid gap-4 lg:grid-cols-2"}>
        {/* —— EchoMuse 分区 —— */}
        <section className="space-y-3">
          <div className="flex items-center gap-2">
            <h3 className="text-sm font-medium">EchoMuse</h3>
            {echoEnabled && echoData !== null ? (
              <span className="text-xs text-muted-foreground">{echoData.devices.length} 台</span>
            ) : null}
          </div>
          {echoLoading ? (
            <p className="text-sm text-muted-foreground">正在读取 EchoMuse 设备列表…</p>
          ) : echoError !== null ? (
            (() => {
              // 字段级 fail-closed：disabled（已编入未启用）→ RestartOnly 文案；
              // not-compiled（HTML 200 归一）→ UnavailableNotice；其余 → danger Alert。
              try {
                // echoQ.error 是字符串（useApiQuery 归一）；用文案特征区分不可用形态。
                if (echoError.includes("echomuse disabled")) {
                  return (
                    <SectionHint
                      line="EchoMuse 未启用。到「EchoMuse 接入」配置开启设备入口并重启服务端。"
                      action={
                        <Button type="button" variant="outline" size="sm" asChild>
                          <Link to="/config">打开接入配置</Link>
                        </Button>
                      }
                    />
                  );
                }
                if (
                  echoError.includes("非 JSON 内容") ||
                  echoError.includes("服务端无此端点")
                ) {
                  return (
                    <UnavailableNotice
                      title="EchoMuse 设备面未编入当前构建"
                      description="当前服务端构建未编入 echomuse-plugin（pure-base / dev 常见）：/api/echomuse/* 请求被管理页 HTML 回退接走，按能力未启用处理。"
                    />
                  );
                }
              } catch {
                // 分类失败落到通用错误行。
              }
              return (
                <div
                  role="alert"
                  className="space-y-2 rounded-lg border border-destructive/30 bg-destructive/5 p-4"
                >
                  <p className="text-sm font-medium text-destructive">EchoMuse 设备列表加载失败</p>
                  <p className="break-words text-xs text-destructive">{echoError}</p>
                  <Button type="button" variant="outline" size="sm" onClick={() => void echoQ.refetch()}>
                    重试
                  </Button>
                </div>
              );
            })()
          ) : echoData !== null && !echoData.enabled ? (
            <SectionHint
              line="EchoMuse 未启用。到「EchoMuse 接入」配置开启设备入口并重启服务端。"
              action={
                <Button type="button" variant="outline" size="sm" asChild>
                  <Link to="/config">打开接入配置</Link>
                </Button>
              }
            />
          ) : echoData !== null && echoData.devices.length === 0 ? (
            <SectionHint
              line="还没有 EchoMuse 设备登记"
              action={
                <p className="text-xs text-muted-foreground">
                  设备首次连上服务端后会出现在这里，经批准后即可管理。
                </p>
              }
            />
          ) : echoData !== null ? (
            <div className="space-y-3">
              {echoData.devices.map((device) => (
                <EchomuseCard
                  key={device.deviceId}
                  device={device}
                  extensions={extensions}
                  busy={busy !== null}
                  onOpen={() => {
                    if (extensions) {
                      setExtensionEmId(device.deviceId);
                    } else {
                      setDetailEmId(device.deviceId);
                    }
                  }}
                  onApprove={() => handleApprove(device)}
                  onDelete={() => handleDeleteEm(device)}
                />
              ))}
            </div>
          ) : null}
        </section>

        {/* —— 涂鸦分区（扩展模式隐藏） —— */}
        {!extensions ? (
          <section className="space-y-3">
            <div className="flex items-center gap-2">
              <h3 className="text-sm font-medium">涂鸦</h3>
              {tuyaEnabled ? (
                <span className="text-xs text-muted-foreground">{tuyaDevices.length} 台</span>
              ) : null}
            </div>
            {tuyaStatusQ.loading && tuyaStatus === null ? (
              <p className="text-sm text-muted-foreground">正在读取涂鸦状态…</p>
            ) : tuyaStatusQ.error !== null ? (
              (() => {
                const error = tuyaStatusQ.error;
                if (error.includes("tuya disabled")) {
                  return (
                    <SectionHint
                      line="涂鸦设备未启用。到「Tuya 接入」配置开启（保存即生效，无需重启）。"
                      action={
                        <Button type="button" variant="outline" size="sm" asChild>
                          <Link to="/config">打开接入配置</Link>
                        </Button>
                      }
                    />
                  );
                }
                if (error.includes("非 JSON 内容") || error.includes("服务端无此端点")) {
                  return (
                    <UnavailableNotice
                      title="涂鸦设备面未编入当前构建"
                      description="当前服务端构建未编入 tuya-plugin（pure-base / dev 常见）：/api/tuya/* 请求被管理页 HTML 回退接走，按能力未启用处理。"
                    />
                  );
                }
                return (
                  <div
                    role="alert"
                    className="space-y-2 rounded-lg border border-destructive/30 bg-destructive/5 p-4"
                  >
                    <p className="text-sm font-medium text-destructive">涂鸦状态加载失败</p>
                    <p className="break-words text-xs text-destructive">{error}</p>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => void tuyaStatusQ.refetch()}
                    >
                      重试
                    </Button>
                  </div>
                );
              })()
            ) : tuyaStatus !== null && !tuyaStatus.enabled ? (
              <SectionHint
                line="涂鸦设备未启用。到「Tuya 接入」配置开启（保存即生效，无需重启）。"
                action={
                  <Button type="button" variant="outline" size="sm" asChild>
                    <Link to="/config">打开接入配置</Link>
                  </Button>
                }
              />
            ) : tuyaStatus !== null && !tuyaStatus.configured ? (
              <SectionHint
                line="凭证未配置：点右上角齿轮录入涂鸦云凭证，或到「Tuya 接入」配置。"
                action={
                  <Button type="button" variant="outline" size="sm" onClick={() => setGearOpen(true)}>
                    打开设备接入配置
                  </Button>
                }
              />
            ) : tuyaDevicesQ.error !== null ? (
              <div
                role="alert"
                className="space-y-2 rounded-lg border border-destructive/30 bg-destructive/5 p-4"
              >
                <p className="text-sm font-medium text-destructive">涂鸦设备加载失败</p>
                <p className="break-words text-xs text-destructive">{tuyaDevicesQ.error}</p>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => void tuyaDevicesQ.refetch()}
                >
                  重试
                </Button>
              </div>
            ) : tuyaDevices.length === 0 ? (
              <SectionHint
                line="还没有涂鸦设备"
                action={
                  <div className="space-y-2">
                    <p className="text-xs text-muted-foreground">
                      录入凭证后点「立即同步」拉取账号下的设备目录；控制全部走服务端局域网直连。
                    </p>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={busy !== null || tuyaStatus?.syncing === true}
                      onClick={() => setGearOpen(true)}
                    >
                      立即同步
                    </Button>
                  </div>
                }
              />
            ) : (
              <div className="space-y-3">
                {tuyaDevices.map((device) => (
                  <TuyaCard
                    key={device.deviceId}
                    device={device}
                    busy={busy !== null}
                    onOpen={() => setDetailTuyaId(device.deviceId)}
                  />
                ))}
              </div>
            )}
            {/* LAN 未登记设备如实展示（不纳管、不落库） */}
            {tuyaStatus !== null && tuyaStatus.enabled && tuyaStatus.lanUnregistered.length > 0 ? (
              <div className="rounded-lg border p-3">
                <p className="text-sm font-medium">局域网未登记设备（{tuyaStatus.lanUnregistered.length}）</p>
                <p className="mt-1 text-xs text-muted-foreground">
                  LAN 扫描发现但未纳管的设备快照：等云同步写入后才进目录。
                </p>
                <div className="mt-2 space-y-1">
                  {tuyaStatus.lanUnregistered.map((item) => (
                    <p key={item.deviceId} className="font-mono text-xs text-muted-foreground">
                      {item.deviceId} · {item.ip !== "" ? item.ip : "IP 未上报"} · {tuyaAgeText(item.ageMs)}
                    </p>
                  ))}
                </div>
              </div>
            ) : null}
            {tuyaStatus !== null && tuyaStatus.enabled && tuyaStatus.storePath !== "" ? (
              <p className="text-xs text-muted-foreground">设备目录存储：{tuyaStatus.storePath}</p>
            ) : null}
            {tuyaStatus !== null && tuyaStatus.enabled && tuyaStatus.lastSync !== null ? (
              <p className="text-xs text-muted-foreground">
                最近同步：{tuyaResultLabel(tuyaStatus.lastSync.result)} ·{" "}
                {tuyaStatus.lastSync.deviceCount} 台 · {tuyaAgeText(tuyaStatus.lastSync.ageMs)}
                {tuyaStatus.lastSync.message !== "" ? ` · ${tuyaStatus.lastSync.message}` : ""}
              </p>
            ) : null}
          </section>
        ) : null}
      </div>

      {/* 删除记录二次确认（§13 实施清单：destructive + 点遮罩不关闭） */}
      <AlertDialog
        open={deleteConfirmId !== null}
        onOpenChange={() => {
          // 忽略 ESC / 遮罩关闭请求：只能走「删除 / 取消」两个按钮。
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>删除设备记录？</AlertDialogTitle>
            <AlertDialogDescription>
              将删除 {deleteTarget?.label || deleteTarget?.deviceId || ""}（
              {deleteTarget?.deviceId ?? ""}）的登记记录；设备下次连接会重新出现。此操作不可撤销。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel
              onClick={() => {
                setDeleteConfirmId(null);
              }}
            >
              取消
            </AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const target = deleteConfirmId;
                setDeleteConfirmId(null);
                if (target !== null) {
                  void runAction(`delete:${target}`, async () => {
                    try {
                      await deleteEchomuseDevice(target);
                      toast.show(`已删除记录 ${target}`, "success");
                      void echoQ.refetch();
                    } catch (error) {
                      if (error instanceof EchomuseApiError && error.status === 404) {
                        toast.show("设备不存在（可能已被删除，刷新列表即可）", "error");
                      } else {
                        toast.show(error instanceof Error ? error.message : String(error), "error");
                      }
                    }
                  });
                }
              }}
            >
              删除
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* 齿轮：设备接入配置（一次性配置；凭证唯一录入路径） */}
      <DeviceAccessConfigDialog
        open={gearOpen}
        onClose={() => setGearOpen(false)}
        echoEnabled={echoData?.enabled ?? null}
        tuyaStatus={tuyaStatus}
        onTuyaChanged={refreshAll}
      />

      {/* EchoMuse 详情：/em/ iframe 嵌入官方页（R4 CDN 直连不 vendor） */}
      <EchoMuseDetailDialog device={detailEm} onClose={() => setDetailEmId(null)} />

      {/* 扩展工具工作区（R11：models / diagnostics / compat 三项） */}
      {extensionEm !== null ? (
        <ExtensionWorkspace
          device={extensionEm}
          open
          onClose={() => setExtensionEmId(null)}
        />
      ) : null}

      {/* 涂鸦详情：设备信息 / 本地别名 / DP 控制 */}
      <TuyaDeviceDialog
        deviceId={detailTuyaId}
        onClose={() => setDetailTuyaId(null)}
        onChanged={refreshAll}
      />

      {/* 扩展模式占位提示（Plus 图标仅在需要时出现；保持 import 被消费） */}
      {extensions && extensionEm === null ? (
        <p className="flex items-center gap-2 text-xs text-muted-foreground">
          <Plus aria-hidden="true" className="size-3" />
          选择一台 EchoMuse 设备打开扩展工具。
        </p>
      ) : null}
    </div>
  );
}
