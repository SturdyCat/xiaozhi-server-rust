/**
 * 插件管理页（W11）：列表 + 上传 + 生命周期 + 详情弹窗 + 动态运行页挂载。
 *
 * 呈现规则逐条移植自 `client/shared/.../PluginsManagePage.kt`（404 行）与
 * `PluginsManageDetailDialog.kt`，文案保留逐字口径：
 * - 三态区分：加载中「正在获取插件清单…」/ 空「暂无插件。…」/ 列表行；
 * - 按钮矩阵（旧端 :217-277 权威）：staged → 注册+移除；registered → 启用（依赖满足
 *   才可用）+移除；active → 停用（无 Active 依赖者才可用）；旧服务端（lifecycle=null）
 *   仅保留动作按钮——**unload 兼容通道 W11 不做**（服务端注明请改用 deactivate/remove）；
 * - register / activate 直接执行；deactivate / remove 走二次确认弹窗（点遮罩不关闭）；
 * - busy 门禁统一：全局 busy 非空禁用所有按钮；操作成功一次性刷新清单（无轮询）；
 * - 详情弹窗是快照静态渲染（零请求）：打开时的清单快照是什么就展示什么。
 *
 * 运行页（`plugin/:pluginId/:entryId`）仅 Active 插件可进：staged/registered 无
 * registration 投影，导航项本就不出现；禁用瞬间停留时回插件管理页（AdminShell :429-433
 * 回退语义）。
 */

import { useMemo, useRef, useState, type ChangeEvent, type ReactElement } from "react";
import { Link, Navigate, useParams } from "react-router-dom";
import { Info, Plus, RefreshCw } from "lucide-react";
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
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { DataState } from "@/components/feedback/DataState";
import { UnavailableNotice } from "@/components/feedback/UnavailableNotice";
import { useToast } from "@/components/feedback/ToastHost";
import {
  PluginUiDocFooter,
  PluginUiDocumentContent,
  type PluginUiActionRunner,
} from "@/components/plugins/PluginUiRenderer";
import { useApiQuery } from "@/lib/useApiQuery";
import {
  activatePlugin,
  deactivatePlugin,
  getPluginsFull,
  registerPlugin,
  removePlugin,
  runPluginAction,
  uploadPluginRaw,
  type PluginRecord,
  type WasmSnapshot,
} from "@/lib/pluginsApi";
import { parsePluginUiDocs, type PluginUiDocument } from "@/lib/pluginUi";

/** 生命周期操作种类（确认弹窗只覆盖 deactivate / remove）。 */
type LifecycleOp = "register" | "activate" | "deactivate" | "remove";

const LIFECYCLE_OP_LABEL: Record<LifecycleOp, string> = {
  register: "注册",
  activate: "启用",
  deactivate: "停用",
  remove: "移除",
};

// ============================================================
// 详情弹窗内容模型（纯组装，零请求；PluginSlots.kt PluginDetailModel 移植）
// ============================================================

interface PluginDetailSection {
  title: string;
  lines: string[];
}

interface PluginDetailModel {
  title: string;
  /** 生命周期一句话（含下一步指引）。 */
  lifecycleLine: string;
  sections: PluginDetailSection[];
}

function lifecycleLineOf(plugin: PluginRecord): string {
  switch (plugin.lifecycle) {
    case "staged":
      return "待注册：上传已完成，注册并启用后才运行业务。";
    case "registered":
      return (
        "已注册·已停用：" +
        (plugin.missingRequirements.length === 0
          ? "满足依赖后启用即可运行。"
          : "存在未满足依赖，启用不可用。")
      );
    case "active":
      return plugin.phase.state === "active"
        ? "运行中。"
        : `运行中（phase：${plugin.phase.state}）。`;
    case null:
      return "运行中（旧服务端投影，无生命周期管理）。";
    default:
      return `状态：${plugin.lifecycle}。`;
  }
}

function buildPluginDetail(
  plugin: PluginRecord,
  uiPages: readonly string[],
): PluginDetailModel {
  const sections: PluginDetailSection[] = [];
  const intro: string[] = [
    `${plugin.id} · v${plugin.version}` +
      (plugin.apiVersion !== "" ? `（宿主契约 v${plugin.apiVersion}）` : ""),
  ];
  if (plugin.description !== "") {
    intro.push(plugin.description);
  }
  if (plugin.implements.length > 0) {
    intro.push(`扩展点：${plugin.implements.join("、")}`);
  }
  if (plugin.permissions.length > 0) {
    intro.push(`权限：${plugin.permissions.join("、")}`);
  }
  sections.push({ title: "功能介绍", lines: intro });
  if (plugin.requires.length > 0 || plugin.dependents.length > 0) {
    const lines = plugin.requires.map(
      (r) =>
        `${r.id} ≥ ${r.minVersion} — ${
          r.satisfied ? "已满足" : "未满足"
        }（当前 ${r.activeVersion ?? "未激活/未装载"}）`,
    );
    if (plugin.dependents.length > 0) {
      lines.push(`被依赖：${plugin.dependents.join("、")}（停用本插件前先停用依赖者）`);
    }
    sections.push({ title: "依赖", lines });
  }
  if (plugin.isActive) {
    if (plugin.registration !== null && plugin.registration.menu.length > 0) {
      sections.push({
        title: "功能入口",
        lines: plugin.registration.menu.map(
          (m) => `${m.label}（${m.slot !== "" ? m.slot : "菜单"}）`,
        ),
      });
    }
    if (plugin.registration !== null && plugin.registration.configFields.length > 0) {
      sections.push({
        title: "配置项",
        lines: plugin.registration.configFields.map(
          (f) =>
            `${f.label}（${f.kind}${f.secret ? " · 密钥" : ""}）` +
            (f.help !== "" ? `：${f.help}` : ""),
        ),
      });
    }
    if (plugin.registration !== null && plugin.registration.actions.length > 0) {
      sections.push({
        title: "动作与用法",
        lines: plugin.registration.actions.map(
          (a) => a.label + (a.help !== "" ? `：${a.help}` : ""),
        ),
      });
    }
    if (uiPages.length > 0) {
      sections.push({ title: "动态页面", lines: [...uiPages] });
    }
  } else {
    sections.push({
      title: "注册状态",
      lines: ["尚未启用：功能入口 / 配置项 / 动作在启用后由插件注册投影下发。"],
    });
  }
  return {
    title: plugin.title,
    lifecycleLine: lifecycleLineOf(plugin),
    sections: sections.filter((section) => section.lines.length > 0),
  };
}

// ============================================================
// 插件列表行
// ============================================================

function PluginRow(props: {
  plugin: PluginRecord;
  busy: string | null;
  onDetails: () => void;
  onLifecycle: (op: LifecycleOp) => void;
  onAction: (actionId: string) => void;
}): ReactElement {
  const { plugin, busy } = props;
  const globalBusy = busy !== null;
  const rowBusy = (key: string): boolean => busy === key;
  return (
    <Card>
      <CardHeader className="flex-row items-start justify-between gap-3 space-y-0">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <CardTitle className="text-base">{plugin.title}</CardTitle>
            <Badge variant="secondary">{plugin.official ? "官方" : "用户"}</Badge>
            <Badge variant="outline">{plugin.lifecycleLabel}</Badge>
          </div>
        </div>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={globalBusy}
            onClick={props.onDetails}
          >
            <Info aria-hidden="true" className="size-4" />
            详情
          </Button>
          {/* 按钮矩阵（PluginsManagePage.kt :217-277 权威） */}
          {plugin.lifecycle === "staged" ? (
            <>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={globalBusy}
                onClick={() => props.onLifecycle("register")}
              >
                {rowBusy(`register:${plugin.id}`) ? "注册中…" : "注册"}
              </Button>
              <Button
                type="button"
                variant="destructive"
                size="sm"
                disabled={globalBusy}
                onClick={() => props.onLifecycle("remove")}
              >
                移除
              </Button>
            </>
          ) : null}
          {plugin.lifecycle === "registered" ? (
            <>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={globalBusy || !plugin.canActivate}
                onClick={() => props.onLifecycle("activate")}
              >
                {rowBusy(`activate:${plugin.id}`) ? "启用中…" : "启用"}
              </Button>
              <Button
                type="button"
                variant="destructive"
                size="sm"
                disabled={globalBusy}
                onClick={() => props.onLifecycle("remove")}
              >
                移除
              </Button>
            </>
          ) : null}
          {plugin.lifecycle === "active" ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={globalBusy || !plugin.canDeactivate}
              onClick={() => props.onLifecycle("deactivate")}
            >
              {rowBusy(`deactivate:${plugin.id}`) ? "停用中…" : "停用"}
            </Button>
          ) : null}
          {plugin.lifecycle === null
            ? // 旧服务端（无 lifecycle 投影）保留动作按钮；unload 兼容通道 W11 不做。
              (plugin.registration?.actions ?? []).map((action) => (
                <Button
                  key={action.id}
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={globalBusy}
                  onClick={() => props.onAction(action.id)}
                >
                  {rowBusy(`${plugin.id}:${action.id}`) ? "执行中…" : action.label}
                </Button>
              ))
            : null}
        </div>
      </CardHeader>
      <CardContent className="space-y-1">
        <p className="text-xs text-muted-foreground">
          {plugin.id}
          {plugin.version !== "" ? ` · v${plugin.version}` : ""} · {plugin.lifecycleLabel}
        </p>
        {plugin.description !== "" ? (
          <p className="break-words text-xs text-muted-foreground">{plugin.description}</p>
        ) : null}
        {plugin.lifecycle === "registered" && plugin.missingRequirements.length > 0 ? (
          <p className="text-xs text-amber-700">
            依赖未满足，无法启用：
            {plugin.missingRequirements.map((r) => r.id).join("、")}
          </p>
        ) : null}
        {plugin.lifecycle === "active" && plugin.dependents.length > 0 ? (
          <p className="text-xs text-amber-700">
            被依赖中：{plugin.dependents.join("、")}
            。需先停用依赖者才能停用本插件。
          </p>
        ) : null}
      </CardContent>
    </Card>
  );
}

// ============================================================
// 插件管理页（列表 + 上传 + 生命周期 + 详情）
// ============================================================

export function PluginsPage(): ReactElement {
  const toast = useToast();
  const query = useApiQuery(getPluginsFull, []);
  const data = query.data;
  const snapshot: WasmSnapshot | null = data?.wasm ?? null;
  const docs = useMemo<Record<string, PluginUiDocument>>(
    () => (snapshot !== null ? parsePluginUiDocs(snapshot) : {}),
    [snapshot],
  );

  // 全局 busy 门禁：key 形态与旧端一致（`${op}:${pluginId}` / `${pluginId}:${actionId}`）。
  const [busy, setBusy] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<string>("");
  const [uploading, setUploading] = useState(false);
  const [uploadPercent, setUploadPercent] = useState(0);
  const [confirmLifecycle, setConfirmLifecycle] = useState<{
    op: "deactivate" | "remove";
    pluginId: string;
  } | null>(null);
  const [detailId, setDetailId] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const showFeedback = (message: string, ok: boolean): void => {
    setFeedback(message);
    toast.show(message, ok ? "success" : "error");
  };

  const runLifecycle = async (op: LifecycleOp, pluginId: string): Promise<void> => {
    const key = `${op}:${pluginId}`;
    setBusy(key);
    try {
      const fns: Record<LifecycleOp, (id: string) => Promise<{ id: string; lifecycle: string }>> = {
        register: registerPlugin,
        activate: activatePlugin,
        deactivate: deactivatePlugin,
        remove: removePlugin,
      };
      await fns[op](pluginId);
      showFeedback(`${LIFECYCLE_OP_LABEL[op]}完成：${pluginId}`, true);
      // 操作成功一次性刷新清单（无轮询）。
      void query.refetch();
    } catch (error) {
      showFeedback(error instanceof Error ? error.message : String(error), false);
    } finally {
      setBusy(null);
    }
  };

  const runRowAction = async (pluginId: string, actionId: string): Promise<void> => {
    const key = `${pluginId}:${actionId}`;
    setBusy(key);
    try {
      await runPluginAction(pluginId, actionId, {});
      toast.show(`动作 ${actionId} 执行成功`, "success");
    } catch (error) {
      toast.show(error instanceof Error ? error.message : String(error), "error");
    } finally {
      setBusy(null);
    }
  };

  const handleFileChosen = (event: ChangeEvent<HTMLInputElement>): void => {
    const file = event.target.files?.[0];
    // 复位 input，让同一文件可以再次选择。
    event.target.value = "";
    if (!file) {
      return;
    }
    setUploading(true);
    setUploadPercent(0);
    uploadPluginRaw(file, (percent) => setUploadPercent(percent))
      .then((result) => {
        let message =
          result.upgraded && result.previousVersion !== null
            ? `已升级并登记 Staged：${result.id} · ${result.previousVersion} → v${result.version}`
            : `已登记 Staged：${result.id} · v${result.version}`;
        if (!result.persisted) {
          message += "；产物持久化不完整（本次运行有效，重启后可能丢失）";
        }
        if (result.reloadError !== null) {
          message += `；Active 原地换版本失败（旧实例保留）：${result.reloadError}`;
        }
        showFeedback(message, true);
        void query.refetch();
      })
      .catch((error: unknown) => {
        showFeedback(error instanceof Error ? error.message : String(error), false);
      })
      .finally(() => {
        setUploading(false);
        setUploadPercent(0);
      });
  };

  const confirmPlugin = confirmLifecycle
    ? (snapshot?.plugins.find((p) => p.id === confirmLifecycle.pluginId) ?? null)
    : null;
  const isRemove = confirmLifecycle?.op === "remove";
  const detailPlugin = detailId
    ? (snapshot?.plugins.find((p) => p.id === detailId) ?? null)
    : null;
  const detailModel = detailPlugin
    ? buildPluginDetail(
        detailPlugin,
        docs[detailPlugin.id]?.pages.map((page) => page.title) ?? [],
      )
    : null;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-lg font-semibold">插件</h2>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy !== null || uploading}
            onClick={() => fileInputRef.current?.click()}
          >
            <Plus aria-hidden="true" className="size-4" />
            添加插件
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={busy !== null || uploading}
            onClick={() => void query.refetch()}
          >
            <RefreshCw aria-hidden="true" className="size-4" />
            刷新清单
          </Button>
        </div>
      </div>
      {/* 上传桥：raw octet-stream 直传，id 由服务端从组件 manifest 探测；体积上限在服务端。 */}
      <input
        ref={fileInputRef}
        type="file"
        accept=".wasm"
        className="hidden"
        onChange={handleFileChosen}
      />
      {uploading ? (
        <div className="space-y-1">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
            <div
              className="h-full rounded-full bg-primary transition-all"
              style={{ width: `${Math.min(Math.max(uploadPercent, 0), 100)}%` }}
            />
          </div>
          <p className="text-xs text-muted-foreground">上传中… {uploadPercent}%</p>
        </div>
      ) : null}
      <p className="text-xs text-muted-foreground">
        上传 WASM 插件后，它会进入列表；详情、依赖与生命周期操作从右侧 actions 进入。
      </p>
      {feedback !== "" ? (
        <p
          className={
            feedback.includes("失败") || feedback.includes("错误")
              ? "text-sm text-destructive"
              : "text-sm"
          }
        >
          {feedback}
        </p>
      ) : null}
      {query.error ? (
        <DataState
          loading={false}
          error={`${query.error}（确认服务端地址可达后点「重试」刷新清单）`}
          onRetry={() => void query.refetch()}
        >
          <></>
        </DataState>
      ) : null}

      {query.loading && data === null ? (
        <p className="text-sm text-muted-foreground">正在获取插件清单…</p>
      ) : data !== null && snapshot === null ? (
        <UnavailableNotice
          title="WASM 插件清单不可用"
          description={`服务端回包没有 wasm 键（ui.mode=${data.uiMode}）：当前构建未编入 wasm-plugins 或宿主未装载，插件管理不可用。静态导航与能力清单不受影响。`}
        />
      ) : snapshot !== null && snapshot.plugins.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          暂无插件。点击右上角 +，选择 .wasm 文件上传。
        </p>
      ) : snapshot !== null ? (
        <div className="space-y-3">
          {snapshot.plugins.map((plugin) => (
            <PluginRow
              key={plugin.id}
              plugin={plugin}
              busy={busy}
              onDetails={() => setDetailId(plugin.id)}
              onLifecycle={(op) => {
                if (op === "deactivate" || op === "remove") {
                  setConfirmLifecycle({ op, pluginId: plugin.id });
                  return;
                }
                void runLifecycle(op, plugin.id);
              }}
              onAction={(actionId) => void runRowAction(plugin.id, actionId)}
            />
          ))}
        </div>
      ) : null}

      {/* 生命周期二次确认（停用 / 移除；点遮罩不关闭） */}
      <AlertDialog
        open={confirmLifecycle !== null}
        onOpenChange={() => {
          // 忽略 ESC / 遮罩关闭请求：只能走「确认 / 取消」两个按钮。
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{isRemove ? "移除插件？" : "停用插件？"}</AlertDialogTitle>
            <AlertDialogDescription>
              {confirmPlugin ? `${confirmPlugin.title}（${confirmPlugin.id}）` : ""}
              {isRemove
                ? "将从服务端移除并删除产物文件。仅停用（非运行）插件可移除；此操作不可撤销。"
                : "将停止运行；注册声明与配置保留，之后可重新启用。若其他插件依赖它，需先停用依赖者。"}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel
              onClick={() => {
                setConfirmLifecycle(null);
              }}
            >
              取消
            </AlertDialogCancel>
            <AlertDialogAction
              variant={isRemove ? "destructive" : "default"}
              onClick={() => {
                const target = confirmLifecycle;
                setConfirmLifecycle(null);
                if (target !== null) {
                  void runLifecycle(target.op, target.pluginId);
                }
              }}
            >
              {isRemove ? "移除" : "停用"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* 详情弹窗：快照静态渲染，零请求；打开时的清单快照是什么就展示什么。 */}
      <Dialog open={detailPlugin !== null} onOpenChange={(open) => {
        if (!open) {
          setDetailId(null);
        }
      }}>
        <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{detailModel?.title ?? ""}</DialogTitle>
          </DialogHeader>
          {detailModel !== null ? (
            <div className="space-y-3">
              <p className="text-xs text-muted-foreground">{detailModel.lifecycleLine}</p>
              {detailModel.sections.map((section) => (
                <div key={section.title} className="rounded-lg border p-3">
                  <p className="text-sm font-medium">{section.title}</p>
                  <div className="mt-1 space-y-1">
                    {section.lines.map((line, index) => (
                      <p key={index} className="break-words text-xs text-muted-foreground">
                        {line}
                      </p>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  );
}

// ============================================================
// 动态运行页（plugin/:pluginId/:entryId 路由挂载）
// ============================================================

/**
 * 声明式运行页：仅消费 Active 插件的 registration.ui；插件未声明 UI 时回退到
 * manifest 字段与动作，业务 UI 不回编进静态管理台。插件缺失 / 非 Active →
 * 回插件管理页（AdminShell :429-433 回退语义）。
 */
export function PluginRuntimeRoute(): ReactElement {
  const params = useParams();
  const pluginId = params.pluginId ?? "";
  const toast = useToast();
  const query = useApiQuery(getPluginsFull, []);
  const data = query.data;
  const snapshot: WasmSnapshot | null = data?.wasm ?? null;
  const [busy, setBusy] = useState<string | null>(null);

  const plugin = pluginId !== "" && snapshot !== null
    ? (snapshot.plugins.find((item) => item.id === pluginId) ?? null)
    : null;

  const docs = useMemo<Record<string, PluginUiDocument>>(
    () => (snapshot !== null ? parsePluginUiDocs(snapshot) : {}),
    [snapshot],
  );
  const doc: PluginUiDocument | null = plugin !== null ? (docs[plugin.id] ?? null) : null;

  const onRun: PluginUiActionRunner = async (actionId, payload) => {
    if (plugin === null) {
      return { actionId, ok: false, message: "插件不可用", raw: "" };
    }
    setBusy(`${plugin.id}:${actionId}`);
    try {
      const result = await runPluginAction(plugin.id, actionId, payload);
      toast.show(`动作 ${actionId} 执行成功`, "success");
      return { actionId, ok: true, message: result.raw || "执行成功", raw: result.raw };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      toast.show(message, "error");
      return { actionId, ok: false, message, raw: "" };
    } finally {
      setBusy(null);
    }
  };

  if (query.loading && data === null) {
    return <p className="text-sm text-muted-foreground">正在获取插件清单…</p>;
  }
  if (query.error) {
    return (
      <DataState loading={false} error={query.error} onRetry={() => void query.refetch()}>
        <></>
      </DataState>
    );
  }
  if (snapshot === null) {
    return (
      <UnavailableNotice
        title="WASM 插件清单不可用"
        description={`服务端回包没有 wasm 键（ui.mode=${data?.uiMode ?? "unknown"}）：当前构建未编入 wasm-plugins 或宿主未装载。`}
      />
    );
  }
  // 仅 active 生命周期可进运行页；禁用瞬间停留时回插件管理页。
  if (plugin === null || !plugin.isActive) {
    return <Navigate to="/plugins" replace />;
  }

  const busyActionId =
    busy !== null && busy.startsWith(`${plugin.id}:`) ? busy.slice(plugin.id.length + 1) : null;

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-lg font-semibold">{plugin.title}</h2>
        <Button type="button" variant="ghost" size="sm" asChild>
          <Link to="/plugins">返回插件管理</Link>
        </Button>
      </div>

      {/* 概述卡（PluginRuntimePage.kt 逐字） */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">{plugin.title}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-1">
          <p className="text-sm">
            {plugin.description !== ""
              ? plugin.description
              : "该页面由插件清单声明，底座不内置业务页面。"}
          </p>
          <p className="text-xs text-muted-foreground">
            {plugin.id} · v{plugin.version} · {plugin.phase.state}
          </p>
          {plugin.implements.length > 0 ? (
            <p className="text-xs text-muted-foreground">
              扩展点：{plugin.implements.join("、")}
            </p>
          ) : null}
          {plugin.permissions.length > 0 ? (
            <p className="text-xs text-muted-foreground">
              权限：{plugin.permissions.join("、")}
            </p>
          ) : null}
        </CardContent>
      </Card>

      {doc !== null ? (
        <>
          <PluginUiDocumentContent
            doc={doc}
            pluginTitle={plugin.title}
            busy={busy !== null}
            busyActionId={busyActionId}
            onRun={onRun}
          />
          <PluginUiDocFooter />
        </>
      ) : (
        <>
          {/* 声明式回退页：manifest 字段卡 + 动作卡 */}
          {plugin.registration !== null && plugin.registration.configFields.length > 0 ? (
            <Card>
              <CardHeader>
                <CardTitle className="text-base">插件配置字段</CardTitle>
              </CardHeader>
              <CardContent className="space-y-1">
                {plugin.registration.configFields.map((field) => (
                  <p key={field.key} className="text-xs text-muted-foreground">
                    {field.label}（{field.key} · {field.kind}）
                    {field.help !== "" ? `：${field.help}` : ""}
                  </p>
                ))}
                <p className="pt-1 text-xs text-muted-foreground">
                  字段由插件 manifest 声明；底座不会为未声明字段生成业务配置。
                </p>
              </CardContent>
            </Card>
          ) : null}
          <Card>
            <CardHeader>
              <CardTitle className="text-base">插件动作</CardTitle>
            </CardHeader>
            <CardContent>
              {(plugin.registration?.actions ?? []).length === 0 ? (
                <p className="text-xs text-muted-foreground">该插件未声明管理动作。</p>
              ) : (
                <div className="flex flex-wrap gap-2">
                  {(plugin.registration?.actions ?? []).map((action) => (
                    <Button
                      key={action.id}
                      type="button"
                      variant={action.danger ? "destructive" : "outline"}
                      size="sm"
                      disabled={busy !== null}
                      onClick={() => void onRun(action.id, {})}
                    >
                      {busyActionId === action.id ? "执行中…" : action.label}
                    </Button>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}
