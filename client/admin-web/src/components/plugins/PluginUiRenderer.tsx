/**
 * 动态插件 UI 壳的渲染层（A2UI v0.1 嵌套组件树 → shadcn/ui）：把
 * [`PluginUiDocument`]{@link ../lib/pluginUi} 的页/节点树分发到既有零件
 * （Card / 正文行 / 提示行 / Button / Input），不手搓官方已有组件。
 *
 * 边界（与解析层契约一致，见 lib/pluginUi.ts）：
 * - 只渲染解析后已通过 fail-closed 过滤的页/树；渲染层不做第二道过滤、也不补节点
 *   （tree 没引用的字段/动作不会在这里出现——声明清单是唯一权威）；
 * - `field`/`form` 是**字段元数据展示**（与声明式回退页同口径）：客户端没有插件配置值
 *   的读取与写入通道，不渲染可编辑输入框，不伪造「已保存」（配置写入是后续独立工作）；
 * - `actions`/`panel` 复用统一动作通道（POST /api/plugins/{id}/actions/{name}），busy
 *   口径与插件管理页一致；danger 动作二次确认弹窗（点遮罩不关闭，与移除/停用同口径），
 *   确认后与普通动作走同一条通道与 payload；
 * - `input` 是页内临时测试输入：值存页面级 React state——不进任何配置草稿、不标脏、
 *   不落配置；页面含 input 时动作 payload = 该页全部输入值对象（未填 = 空串），
 *   无输入页保持空对象（现状不变）；
 * - `result` 展示该页最近一次动作调用的 ok/err + 原文回包；页按文档顺序整页渲染，
 *   `kind` 仅作展示标注。
 */

import { useState, type ReactElement } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
import { cn } from "@/lib/utils";
import type { PluginAction, PluginConfigField } from "@/lib/pluginsApi";
import { A2UI_SUPPORTED_VERSION } from "@/lib/pluginUi";
import type { PluginUiDocument, PluginUiNode, PluginUiPage } from "@/lib/pluginUi";

/** 一次动作调用的 UI 运行态结果（非持久契约）；`raw` 是服务端回包原文。 */
export interface PluginUiActionOutcome {
  /** 触发本次调用的动作 id（result 节点按「成功 · {actionId}：{message}」展示）。 */
  actionId: string;
  ok: boolean;
  message: string;
  raw: string;
}

/** 渲染器与页面的动作通道：由 PluginsPage 提供（统一 busy 门禁与 toast）。 */
export type PluginUiActionRunner = (
  actionId: string,
  payload: Record<string, string>,
) => Promise<PluginUiActionOutcome>;

export interface PluginUiRendererProps {
  doc: PluginUiDocument;
  /** 危险动作确认文案与 toast 语境用。 */
  pluginTitle: string;
  /** 全局 busy：true 时禁用所有按钮（与插件管理页口径一致）。 */
  busy: boolean;
  /** 当前正在执行的动作 id（null = 无）；对应按钮显示「执行中…」。 */
  busyActionId: string | null;
  onRun: PluginUiActionRunner;
}

/** 渲染一份动态 UI 文档（调用方保证 doc 非空；页/树已在解析期 fail-closed）。 */
export function PluginUiDocumentContent(props: PluginUiRendererProps): ReactElement {
  return (
    <div className="space-y-4">
      {props.doc.pages.map((page) => (
        <PluginUiPageView key={page.id} page={page} {...props} />
      ))}
    </div>
  );
}

/** 文档页的收尾说明（文档模式专属；回退页不渲染）。 */
export function PluginUiDocFooter(): ReactElement {
  return (
    <p className="text-xs text-muted-foreground">
      页面布局由插件 registration.ui（A2UI {A2UI_SUPPORTED_VERSION} 嵌套组件树）声明：
      容器只做布局，字段/动作只来自注册清单，input 仅页内临时输入，底座不生成业务表单。
    </p>
  );
}

function PluginUiPageView({
  page,
  pluginTitle,
  busy,
  busyActionId,
  onRun,
}: PluginUiRendererProps & { page: PluginUiPage }): ReactElement {
  // 页面级运行态（纯展示；不进任何配置草稿、不标脏、不落配置）
  const [inputValues, setInputValues] = useState<Record<string, string>>({});
  const [lastResult, setLastResult] = useState<PluginUiActionOutcome | null>(null);
  const [confirmDanger, setConfirmDanger] = useState<PluginAction | null>(null);

  /** 动作 payload：页面含 input → 全部输入值对象（未填 = 空串）；否则空对象（现状不变）。 */
  const payloadOf = (): Record<string, string> => {
    const payload: Record<string, string> = {};
    for (const id of page.inputIds) {
      payload[id] = inputValues[id] ?? "";
    }
    return payload;
  };

  const execute = async (action: PluginAction): Promise<void> => {
    const outcome = await onRun(action.id, payloadOf());
    setLastResult(outcome);
  };

  const handleAction = (action: PluginAction): void => {
    if (action.danger) {
      setConfirmDanger(action);
      return;
    }
    void execute(action);
  };

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle className="text-base">{page.title}</CardTitle>
          {page.kind !== "" ? (
            <p className="text-xs text-muted-foreground">页面类型：{page.kind}</p>
          ) : null}
        </CardHeader>
        <CardContent>
          <PluginUiNodeView
            node={page.tree}
            busy={busy}
            busyActionId={busyActionId}
            inputValues={inputValues}
            onInput={(id, value) => setInputValues((previous) => ({ ...previous, [id]: value }))}
            onAction={handleAction}
            lastResult={lastResult}
          />
        </CardContent>
      </Card>

      {/* 危险动作二次确认（元数据取注册声明；点遮罩不关闭——onOpenChange 忽略关闭请求） */}
      <AlertDialog
        open={confirmDanger !== null}
        onOpenChange={() => {
          // 忽略 ESC / 遮罩关闭请求：只能走「执行 / 取消」两个按钮。
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>执行危险动作？</AlertDialogTitle>
            <AlertDialogDescription>
              {pluginTitle} · {confirmDanger?.label ?? ""}
              {confirmDanger?.help ? `：${confirmDanger.help}` : ""}
              。该动作由插件声明为危险操作，请确认后执行。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel
              onClick={() => {
                setConfirmDanger(null);
              }}
            >
              取消
            </AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const action = confirmDanger;
                setConfirmDanger(null);
                if (action !== null) {
                  void execute(action);
                }
              }}
            >
              执行
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

/** 节点分发：容器递归，叶子落到 shadcn 零件（tree 已 fail-closed，这里不再过滤）。 */
function PluginUiNodeView(props: {
  node: PluginUiNode;
  busy: boolean;
  busyActionId: string | null;
  inputValues: Record<string, string>;
  onInput: (id: string, value: string) => void;
  lastResult: PluginUiActionOutcome | null;
  onAction: (action: PluginAction) => void;
}): ReactElement {
  const { node } = props;
  switch (node.type) {
    case "card":
      return (
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">{node.title}</CardTitle>
          </CardHeader>
          <CardContent>
            <PluginUiNodeView {...props} node={node.child} />
          </CardContent>
        </Card>
      );
    case "column":
      return (
        <div className="space-y-3">
          {node.children.map((child, index) => (
            <PluginUiNodeView key={index} {...props} node={child} />
          ))}
        </div>
      );
    case "text":
      return <p className="text-sm leading-relaxed">{node.text}</p>;
    case "hint":
      return <p className="text-xs text-muted-foreground">{node.text}</p>;
    case "field":
      return <PluginUiFieldRow field={node.field} />;
    case "form":
      return (
        <div className="space-y-1">
          {node.fields.map((field) => (
            <PluginUiFieldRow key={field.key} field={field} />
          ))}
          <p className="pt-1 text-xs text-muted-foreground">
            字段由插件 registration 声明；当前版本展示声明元数据，配置值读写随后续插件面开放。
          </p>
        </div>
      );
    case "actions":
    case "panel":
      // 旧扁平形态（panel）与 actions 同语义：按钮组（元数据已取注册声明）。
      return <PluginUiActions {...props} actions={node.actions} />;
    // 页内临时输入：值写页面级 state，不进任何配置草稿。
    case "input":
      return (
        <div className="space-y-1.5">
          <Label className="text-sm" htmlFor={`plugin-input-${node.id}`}>
            {node.label}
          </Label>
          <Input
            id={`plugin-input-${node.id}`}
            value={props.inputValues[node.id] ?? ""}
            placeholder={node.placeholder !== "" ? node.placeholder : undefined}
            onChange={(event) => props.onInput(node.id, event.target.value)}
          />
        </div>
      );
    case "result":
      return <PluginUiResultView result={props.lastResult} />;
  }
}

/** 声明字段的元数据展示行（不含值：客户端无插件配置读取通道，不伪造）。 */
function PluginUiFieldRow({ field }: { field: PluginConfigField }): ReactElement {
  const secretNote = field.secret ? " · 密钥字段（值不回显）" : "";
  const help = field.help !== "" ? `：${field.help}` : "";
  return (
    <p className="text-xs text-muted-foreground">
      {field.label}（{field.kind}
      {secretNote}）{help}
    </p>
  );
}

/** 动作按钮组：元数据来自注册声明；danger 动作由页面侧二次确认后执行。 */
function PluginUiActions({
  actions,
  busy,
  busyActionId,
  onAction,
}: {
  actions: readonly PluginAction[];
  busy: boolean;
  busyActionId: string | null;
  onAction: (action: PluginAction) => void;
}): ReactElement {
  if (actions.length === 0) {
    return <></>;
  }
  return (
    <div className="flex flex-wrap gap-2">
      {actions.map((action) => (
        <Button
          key={action.id}
          type="button"
          variant={action.danger ? "destructive" : "outline"}
          size="sm"
          disabled={busy}
          onClick={() => onAction(action)}
        >
          {busyActionId === action.id ? "执行中…" : action.label}
        </Button>
      ))}
    </div>
  );
}

/** `result` 节点：该页最近一次动作调用的 ok/err + 原文回包（尚未执行时说明占位语义）。 */
function PluginUiResultView({ result }: { result: PluginUiActionOutcome | null }): ReactElement {
  if (result === null) {
    return (
      <p className="text-xs text-muted-foreground">
        该页尚未执行动作；动作完成后此处显示最近一次调用结果与回包原文。
      </p>
    );
  }
  return (
    <div className="space-y-1">
      <p className={cn("text-sm", result.ok ? "text-foreground" : "text-destructive")}>
        {result.ok ? "成功" : "失败"} · {result.actionId}：{result.message}
      </p>
      {result.raw !== "" ? (
        <p className="break-all text-xs text-muted-foreground">回包原文：{result.raw}</p>
      ) : null}
    </div>
  );
}
