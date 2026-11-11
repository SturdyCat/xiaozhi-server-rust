/**
 * `[memory]` 记忆配置段（W8 / Config C5）。
 *
 * 字段清单与默认值：`plugins/memory/config.rs:142-312`（14 个标量 +
 * retention{keep/recent_turns/retention_days/dry_run} + extractor{api_base/api_key/model/
 * temperature f32 0.1} + rag{docs_dir/chunk_size 512/overlap 64/top_k 4}；
 * engine 闭合枚举 graph/store/rag/none，旧键 backend 经 serde alias 仍可读）。
 * advanced 归属：`plugins/registry/projection.rs:42-92`——**memory.enabled 是唯一
 * 非高级字段**，其余全部 `advanced`（旧端 Kotlin 卡把 recall_enabled 等放在简单模式，
 * 与 schema 的 advanced_fields 不一致：按 schema 全量暴露，不照抄旧端口径）。
 *
 * 四组不可混淆的动作（对齐旧端 `ConfigMemoryCard.kt:27-42` 的表）：
 * 1. 刷新状态 `GET /api/memory/status`：只读投影；**无自动轮询**，只有手动刷新。
 * 2. 立即维护 / 演练保留策略 `POST /api/memory/maintain`：前端**显式传全**
 *    `{force:true, dry_run}`；演练（dry_run=true）未删除任何数据，文案必须如实区分。
 * 3. 测试召回 `POST /api/memory/recall`：text 必填（空串不发请求）。
 * 4. 清空 `POST /api/memory/clear`：**两段式强确认**；scope / all 变化即作废上一段
 *    确认（避免「确认的是 A、删的是 B」）；两者皆无服务端 400，前端不发。
 *
 * 与旧端的刻意差异（文件头声明，均为**新增行为，非旧端移植**）：
 * - busy 门禁统一增强：任一动作在飞，本段所有动作按钮全部禁用（旧端无此约束）。
 * - 「测试召回的一句话 / 召回作用域 / 清空的作用域 / 清空全部设备」是**界面本地状态**，
 *   不进配置草稿（旧端把测试文本误标 dirty，污染未保存计数，这里不照抄）。
 * - RAG 重建索引：异步后台任务，服务端 message **原样透出**，绝不因 ok:true 显示
 *   「已完成」；进度请手动点「刷新状态」看 rag 段（前端不做自动轮询）。
 *
 * 呈现纪律：lexical_only / capability.phase 降级如实呈现（不是故障，不伪装成正常）；
 * aiui_note 原样透出；rag 段按存在性判断（graph/store 没有 rag 键，不是 0 值）。
 * 端点契约（与 W7 相反的三点，见 `soulMemoryApi.ts` 头注释）：7 个端点全部
 * native-plugins 门控 + 无鉴权 + 裸 Value 请求体；形状不符按能力未启用处理
 * （`UnavailableNotice`），绝不伪装成空数据。
 */

import { useCallback, useEffect, useRef, useState, type ReactElement } from "react";
import { LoaderCircle } from "lucide-react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { UnavailableNotice } from "@/components/feedback/UnavailableNotice";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Input } from "@/components/ui/input";
import { FormNotice } from "@/components/form/FormNotice";
import { NumberField } from "@/components/form/NumberField";
import { SecretField } from "@/components/form/SecretField";
import { SelectField } from "@/components/form/SelectField";
import { SwitchField } from "@/components/form/SwitchField";
import { TextField } from "@/components/form/TextField";
import type { SelectOption } from "@/components/form/types";
import { Label } from "@/components/ui/label";
import { useAdvancedMode } from "@/context/AdvancedModeContext";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import { formatDateTime } from "@/lib/utils";
import {
  clearMemory,
  getMemoryStatus,
  isSoulMemoryUnavailable,
  maintainMemory,
  recallMemory,
  reindexRag,
  soulMemoryErrorMessage,
  type MemoryClearResult,
  type MemoryMaintainResult,
  type MemoryRecallResult,
  type MemoryStatus,
} from "@/lib/soulMemoryApi";

const ENGINE_OPTIONS: SelectOption[] = [
  { value: "graph", label: "内置图记忆（graph）" },
  { value: "store", label: "纯存储（store）" },
  { value: "rag", label: "本地文档 RAG（rag）" },
  { value: "none", label: "不生效（none，显式停用）" },
];

const INJECT_POSITION_OPTIONS: SelectOption[] = [
  { value: "after_history", label: "历史之后（after_history，默认）" },
  { value: "before_history", label: "历史之前（before_history）" },
];

const RETENTION_KEEP_OPTIONS: SelectOption[] = [
  { value: "all", label: "全部保留（all，默认）" },
  { value: "referenced", label: "只留被记忆引用的原文（referenced）" },
  { value: "recent", label: "最近 N 轮之外的未引用原文（recent）" },
];

/** 库文件大小的人类可读格式。 */
function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
}

/** 状态卡（只读投影）；`status === null` 时渲染占位说明。 */
function StatusCard({ status }: { status: MemoryStatus | null }): ReactElement {
  if (status === null) {
    return (
      <p className="text-sm text-muted-foreground">
        尚未读取状态；点上方「刷新状态」查看库大小、条数、抽取器与作用域。
      </p>
    );
  }
  return (
    <div className="space-y-2 text-sm text-muted-foreground">
      <p>
        引擎 {status.engine === "" ? "未知" : status.engine}
        {status.active !== null ? (status.active ? "（生效中）" : "（未生效）") : ""}
        {status.dbPath !== null ? ` · ${status.dbPath}` : ""}
        {status.dbBytes !== null ? `（${formatBytes(status.dbBytes)}）` : ""}
      </p>
      <p>
        记忆 {status.memories ?? "?"} 条 · 原文消息 {status.messages ?? "?"} 条 · 轮次{" "}
        {status.turns ?? "?"} · 待抽取 {status.pending ?? "?"} · 隔离 {status.quarantined ?? "?"}
        {status.stored !== null ? ` · 已入库原文 ${status.stored} 条` : ""}
      </p>
      {status.scopes.length > 0 ? (
        <p>
          作用域：
          {status.scopes
            .map((item) => `${item.scope}（${item.memories} 条）`)
            .join("、")}
        </p>
      ) : null}
      {status.capabilityPhase !== null ? <p>能力阶段：{status.capabilityPhase}</p> : null}
      {status.lexicalOnly === true ? (
        <FormNotice tone="info">
          当前为本地词法召回（lexical_only：未配置 embedding）。这不是故障：召回仍会工作，但只按关键词匹配、语义泛化能力有限。
        </FormNotice>
      ) : null}
      {status.capabilityLastError !== null ? (
        <FormNotice tone="danger">能力降级原因（服务端 last_error）：{status.capabilityLastError}</FormNotice>
      ) : null}
      {status.aiuiNote !== null ? (
        <FormNotice tone="warning">{status.aiuiNote}</FormNotice>
      ) : null}
      {status.rag !== null ? (
        <div className="space-y-2">
          <p>
            文档 RAG：{status.rag.docs} 个文档 · {status.rag.chunks} 个分块
            {status.rag.lastBuildAtMs !== null
              ? ` · 上次构建 ${formatDateTime(status.rag.lastBuildAtMs)}`
              : ""}
            {status.rag.lastBuildDocs !== null && status.rag.lastBuildChunks !== null
              ? `（${status.rag.lastBuildDocs} 文档 / ${status.rag.lastBuildChunks} 分块）`
              : ""}
            {status.rag.building ? " · 索引重建中…" : ""}
          </p>
          {status.rag.lastError !== null ? (
            <FormNotice tone="danger">RAG 上次构建失败：{status.rag.lastError}</FormNotice>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

export function MemorySection(): ReactElement {
  const { advancedMode } = useAdvancedMode();
  const { getField } = useConfigDraft();

  const enabled = getField("memory.enabled") === true;
  const engine = (() => {
    const value = getField("memory.engine");
    return typeof value === "string" ? value : "";
  })();

  // ---- 统一 busy 门禁（新增行为，非旧端移植）：任一动作在飞，全部动作按钮禁用 ----
  const mountedRef = useRef(true);
  const [statusBusy, setStatusBusy] = useState(false);
  const [maintainBusy, setMaintainBusy] = useState(false);
  const [recallBusy, setRecallBusy] = useState(false);
  const [clearBusy, setClearBusy] = useState(false);
  const [reindexBusy, setReindexBusy] = useState(false);
  const anyBusy = statusBusy || maintainBusy || recallBusy || clearBusy || reindexBusy;

  // ---- 状态卡（只读；无自动轮询，仅手动刷新）----
  const [status, setStatus] = useState<MemoryStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [statusUnavailable, setStatusUnavailable] = useState(false);
  const [statusUnavailableMessage, setStatusUnavailableMessage] = useState<string | null>(null);

  // ---- 维护（演练 / 实跑）----
  const [maintainResult, setMaintainResult] = useState<MemoryMaintainResult | null>(null);
  const [maintainError, setMaintainError] = useState<string | null>(null);

  // ---- 测试召回（界面本地状态，不进配置草稿）----
  const [testText, setTestText] = useState("");
  const [testScope, setTestScope] = useState("");
  const [recallResult, setRecallResult] = useState<MemoryRecallResult | null>(null);
  const [recallError, setRecallError] = useState<string | null>(null);

  // ---- 清空（两段式强确认；scope / all 变化即作废确认）----
  const [clearScope, setClearScope] = useState("");
  const [clearAll, setClearAll] = useState(false);
  const [clearArmed, setClearArmed] = useState(false);
  const [clearResult, setClearResult] = useState<MemoryClearResult | null>(null);
  const [clearError, setClearError] = useState<string | null>(null);

  // ---- RAG 重建索引 ----
  const [reindexMessage, setReindexMessage] = useState<string | null>(null);
  const [reindexError, setReindexError] = useState<string | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  /** 手动刷新状态；形状不符按能力未启用处理，不伪装成空数据。 */
  const loadStatus = useCallback(async (): Promise<void> => {
    setStatusBusy(true);
    try {
      const result = await getMemoryStatus();
      if (!mountedRef.current) {
        return;
      }
      setStatus(result);
      setStatusUnavailable(false);
      setStatusUnavailableMessage(null);
      setStatusError(null);
    } catch (error) {
      if (!mountedRef.current) {
        return;
      }
      setStatus(null);
      if (isSoulMemoryUnavailable(error)) {
        setStatusUnavailable(true);
        setStatusUnavailableMessage(soulMemoryErrorMessage(error));
        setStatusError(null);
      } else {
        setStatusError(soulMemoryErrorMessage(error));
      }
    } finally {
      if (mountedRef.current) {
        setStatusBusy(false);
      }
    }
  }, []);

  /** 立即维护 / 演练保留策略：显式传全 {force:true, dry_run}；文案如实区分演练与实跑。 */
  const runMaintain = (dryRun: boolean): void => {
    setMaintainBusy(true);
    setMaintainResult(null);
    setMaintainError(null);
    void (async () => {
      try {
        const result = await maintainMemory(dryRun);
        if (!mountedRef.current) {
          return;
        }
        setMaintainResult(result);
      } catch (error) {
        if (!mountedRef.current) {
          return;
        }
        setMaintainError(soulMemoryErrorMessage(error));
      } finally {
        if (mountedRef.current) {
          setMaintainBusy(false);
        }
      }
    })();
  };

  /** 测试召回：text 必填（空串不发请求）；scope 留空 = 跨全部作用域。 */
  const handleRecall = (): void => {
    const text = testText.trim();
    if (text === "") {
      setRecallError("请先填写试召回的一句话（服务端对空 text 直接 400）。");
      return;
    }
    setRecallBusy(true);
    setRecallResult(null);
    setRecallError(null);
    const scope = testScope.trim();
    void (async () => {
      try {
        const result = await recallMemory(scope === "" ? { text } : { text, scope });
        if (!mountedRef.current) {
          return;
        }
        setRecallResult(result);
      } catch (error) {
        if (!mountedRef.current) {
          return;
        }
        setRecallError(soulMemoryErrorMessage(error));
      } finally {
        if (mountedRef.current) {
          setRecallBusy(false);
        }
      }
    })();
  };

  /** 清空：第一次点击只置位（两段式）；scope / all 变化即复位由各自的 setter 完成。 */
  const handleClear = (): void => {
    if (!clearArmed) {
      setClearArmed(true);
      setClearError(null);
      setClearResult(null);
      return;
    }
    if (!clearAll && clearScope.trim() === "") {
      setClearArmed(false);
      setClearError(
        "必须填写要清空的作用域，或打开「清空全部设备」；两者皆无服务端会直接拒绝（400），前端不发这次请求。",
      );
      return;
    }
    setClearBusy(true);
    setClearError(null);
    void (async () => {
      try {
        const result = await clearMemory(
          clearAll ? { all: true } : { scope: clearScope.trim() },
        );
        if (!mountedRef.current) {
          return;
        }
        setClearResult(result);
        setClearArmed(false);
      } catch (error) {
        if (!mountedRef.current) {
          return;
        }
        setClearError(soulMemoryErrorMessage(error));
      } finally {
        if (mountedRef.current) {
          setClearBusy(false);
        }
      }
    })();
  };

  /** 重建 RAG 索引：异步任务，message 原样透出；绝不因 ok:true 显示「已完成」。 */
  const handleReindex = (): void => {
    setReindexBusy(true);
    setReindexMessage(null);
    setReindexError(null);
    void (async () => {
      try {
        const result = await reindexRag();
        if (!mountedRef.current) {
          return;
        }
        setReindexMessage(
          `${result.message}（进度不会自动刷新：请稍后点「刷新状态」查看 rag 段的 building / 分块数。）`,
        );
      } catch (error) {
        if (!mountedRef.current) {
          return;
        }
        setReindexError(soulMemoryErrorMessage(error));
      } finally {
        if (mountedRef.current) {
          setReindexBusy(false);
        }
      }
    })();
  };

  const engineLabel =
    ENGINE_OPTIONS.find((option) => option.value === engine)?.label ?? engine;

  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="memory" />

      <SectionCard
        title="记忆（[memory]）"
        description="本地图记忆：对话自动积累、召回注入 system prompt。字段来自 plugins/memory/config.rs:142-312。"
      >
        <div className="divide-y divide-border">
          <SwitchField
            path="memory.enabled"
            label="启用记忆"
            hint="关闭后不再召回、不再抽取，已有数据保留在库文件里。"
          />
          {enabled ? (
            !advancedMode ? (
              <p className="py-2 text-xs text-muted-foreground">
                当前引擎：{engineLabel === "" ? "未设置（服务端默认 graph）" : engineLabel}
                ；沿用已保存的召回与写入策略。详细设置和维护操作在高级模式查看。
              </p>
            ) : null
          ) : (
            <p className="py-2 text-xs text-muted-foreground">
              未启用：不再召回、不再抽取，已有数据保留在库文件里。
            </p>
          )}
        </div>
      </SectionCard>

      {enabled ? (
        <>
          <SectionCard
            title="状态与动作"
            description="刷新状态是只读投影（无自动轮询）；维护 / 清空直接作用于服务端数据，与配置保存无关。"
          >
            <div className="space-y-4">
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={anyBusy}
                  onClick={() => void loadStatus()}
                >
                  {statusBusy ? (
                    <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
                  ) : null}
                  刷新状态
                </Button>
                {advancedMode ? (
                  <>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={anyBusy}
                      onClick={() => runMaintain(false)}
                    >
                      {maintainBusy ? (
                        <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
                      ) : null}
                      立即维护
                    </Button>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={anyBusy}
                      onClick={() => runMaintain(true)}
                    >
                      演练保留策略
                    </Button>
                  </>
                ) : null}
              </div>

              {statusUnavailable ? (
                <div className="space-y-2">
                  <UnavailableNotice
                    title="记忆能力未启用"
                    description={
                      statusUnavailableMessage ??
                      "服务端未挂载 /api/memory/*（通常是构建未启用 native-plugins）。"
                    }
                  />
                  <FormNotice tone="warning">
                    记忆端点未启用或被管理页 HTML 回退接走：状态与维护 / 召回 / 清空动作均不可用，但 [memory] 配置字段仍可正常保存。
                  </FormNotice>
                </div>
              ) : null}
              {statusError !== null ? (
                <FormNotice tone="danger">读取记忆状态失败：{statusError}</FormNotice>
              ) : null}

              <StatusCard status={status} />

              {maintainResult !== null ? (
                <FormNotice tone={maintainResult.dryRun ? "info" : "warning"}>
                  {maintainResult.dryRun
                    ? `演练完成（未删除任何数据）：若实跑，预计释放 ${maintainResult.released ?? "?"} 条、清理 ${maintainResult.deleted ?? "?"} 条；确认无误后请用「立即维护」实跑。`
                    : `维护完成：释放 ${maintainResult.released ?? "?"} 条、删除 ${maintainResult.deleted ?? "?"} 条。如需最新条数请点「刷新状态」。`}
                </FormNotice>
              ) : null}
              {maintainError !== null ? (
                <FormNotice tone="danger">维护失败：{maintainError}</FormNotice>
              ) : null}

              {advancedMode ? (
                <>
                  {/* ===== 测试召回（输入为界面本地状态，不进配置草稿）===== */}
                  <div className="grid gap-2 border-t border-border pt-3">
                    <div className="space-y-1">
                      <Label className="text-sm font-medium">试召回的一句话</Label>
                      <Input
                        className="max-w-md"
                        value={testText}
                        placeholder="用一句用户可能说的话验证记忆是否在工作"
                        onChange={(event) => setTestText(event.target.value)}
                      />
                    </div>
                    <div className="space-y-1">
                      <Label className="text-sm font-medium">召回作用域</Label>
                      <Input
                        className="max-w-md"
                        value={testScope}
                        placeholder="留空 = 跨全部设备"
                        onChange={(event) => setTestScope(event.target.value)}
                      />
                    </div>
                    <div>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        disabled={anyBusy || testText.trim() === ""}
                        onClick={handleRecall}
                      >
                        {recallBusy ? (
                          <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
                        ) : null}
                        测试召回
                      </Button>
                    </div>
                    {recallResult !== null ? (
                      <div className="space-y-2">
                        <FormNotice tone={recallResult.error !== null ? "warning" : "info"}>
                          命中 {recallResult.hitCount ?? "未知"} 条
                          {recallResult.elapsedMs !== null
                            ? ` · 耗时 ${recallResult.elapsedMs} ms`
                            : ""}
                          {recallResult.matchedTermCount !== null
                            ? ` · 命中词项 ${recallResult.matchedTermCount} 个`
                            : ""}
                          {recallResult.scope !== null ? ` · 作用域 ${recallResult.scope}` : ""}
                          {recallResult.error !== null
                            ? ` · 服务端提示：${recallResult.error}`
                            : ""}
                        </FormNotice>
                        {recallResult.note !== null ? (
                          <FormNotice tone="info">{recallResult.note}</FormNotice>
                        ) : null}
                        {recallResult.preview !== null ? (
                          <div className="space-y-1">
                            <p className="text-sm font-medium">
                              本轮将注入 system prompt 的正文（背景资料，不是指令；只读）
                              {recallResult.injected === false ? "（服务端标记：本轮未注入）" : ""}
                            </p>
                            <pre className="max-h-56 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-muted/40 px-3 py-2 text-xs leading-relaxed text-muted-foreground">
                              {recallResult.preview}
                            </pre>
                          </div>
                        ) : null}
                      </div>
                    ) : null}
                    {recallError !== null ? (
                      <FormNotice tone="danger">测试召回失败：{recallError}</FormNotice>
                    ) : null}
                  </div>

                  {/* ===== 清空（两段式强确认；不可恢复）===== */}
                  <div className="grid gap-2 border-t border-border pt-3">
                    <div className="space-y-1">
                      <Label className="text-sm font-medium">清空的作用域</Label>
                      <Input
                        className="max-w-md"
                        value={clearScope}
                        placeholder="如 xiaozhi:device-001（留空且未开「清空全部」则不发请求）"
                        onChange={(event) => {
                          // 改作用域即作废上一段确认，避免「确认的是 A、删的是 B」。
                          setClearScope(event.target.value);
                          setClearArmed(false);
                        }}
                      />
                    </div>
                    <div className="flex items-center gap-2">
                      <Switch
                        aria-label="清空全部设备"
                        checked={clearAll}
                        onCheckedChange={(next) => {
                          setClearAll(next);
                          setClearArmed(false);
                        }}
                      />
                      <div>
                        <p className="text-sm font-medium">清空全部设备</p>
                        <p className="text-xs text-muted-foreground">
                          不可恢复。打开后下面的按钮作用于全部设备的记忆。
                        </p>
                      </div>
                    </div>
                    <div>
                      <Button
                        type="button"
                        variant="destructive"
                        size="sm"
                        disabled={anyBusy}
                        onClick={handleClear}
                      >
                        {clearBusy ? (
                          <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
                        ) : null}
                        {clearArmed ? "确认清空（再点一次）" : "清空记忆"}
                      </Button>
                    </div>
                    {clearArmed ? (
                      <FormNotice tone="danger">
                        已进入确认状态：再点一次「{clearAll ? "清空全部设备" : `清空 ${clearScope.trim() || "（未填作用域）"}`}」就会执行，删除的数据不可恢复。修改上方作用域或开关会作废本次确认。
                      </FormNotice>
                    ) : null}
                    {clearResult !== null ? (
                      <FormNotice tone="warning">
                        {clearResult.message ??
                          `已清空${clearResult.all ? "全部设备" : `作用域 ${clearResult.scope ?? "?"}`}的记忆（删除 ${clearResult.deleted ?? "?"} 条）。`}
                        不可恢复；如需最新条数请点「刷新状态」。
                      </FormNotice>
                    ) : null}
                    {clearError !== null ? (
                      <FormNotice tone="danger">清空失败：{clearError}</FormNotice>
                    ) : null}
                  </div>
                </>
              ) : null}
            </div>
          </SectionCard>

          {advancedMode ? (
            <>
              <SectionCard
                title="引擎与存储"
                description="engine 为闭合枚举 graph / store / rag / none（旧键 backend 仍可读，保存后写 engine）；未知值界面只告警、不自动纠正。"
              >
                <div className="divide-y divide-border">
                  <SelectField
                    path="memory.engine"
                    label="记忆引擎"
                    advanced
                    options={ENGINE_OPTIONS}
                    hint="graph = 内置图记忆（本地 SQLite 图谱）；store = 纯存储（只存原文，零 LLM 零召回）；rag = 本地文档 RAG（扫描目录建词法索引）；none = 显式停用（保留配置但不起作用）。"
                  />
                  <TextField
                    path="memory.db_path"
                    label="库文件路径"
                    advanced
                    placeholder="/data/data.db"
                    hint="SQLite 库路径，默认 /data/data.db（与配置库共用同一文件）；容器部署请放在挂载卷内，否则重建容器即丢。"
                  />
                  <NumberField
                    path="memory.db_busy_timeout_ms"
                    label="SQLite 写锁等待（毫秒）"
                    advanced
                    min={0}
                    placeholder="5000"
                    hint="u64，默认 5000；与 DSH 进程共用同一 db 文件时尤其重要。"
                  />
                  <TextField
                    path="memory.scope_prefix"
                    label="作用域前缀"
                    advanced
                    placeholder="xiaozhi"
                    hint="scope = <prefix>:<device_id>（跨会话稳定，是「记忆」成立的前提）；默认 xiaozhi。"
                  />
                </div>
              </SectionCard>

              <SectionCard
                title="召回与抽取"
                description="召回 / 抽取开关与预算；数值全部为整数（u32 / u64 / usize）。"
              >
                <div className="divide-y divide-border">
                  <SwitchField
                    path="memory.recall_enabled"
                    label="启用召回"
                    advanced
                    hint="关掉 = 只写不读，便于先积累数据。"
                  />
                  <SwitchField
                    path="memory.extraction_enabled"
                    label="启用后台抽取"
                    advanced
                    hint="关闭后只记原文、不抽取结构化记忆（先积累再决定时用）；零辅助 LLM 调用。"
                  />
                  <NumberField
                    path="memory.fresh_turn_count"
                    label="近况轮数"
                    advanced
                    min={0}
                    placeholder="5"
                    hint="u32，默认 5；最近 N 轮原文已在本轮上下文里，不再重复召回。"
                  />
                  <NumberField
                    path="memory.recall_max_nodes"
                    label="最多召回条数"
                    advanced
                    min={0}
                    placeholder="6"
                    hint="usize，默认 6；每次最多召回几条轮次记忆。"
                  />
                  <NumberField
                    path="memory.recall_max_tokens"
                    label="注入 token 预算"
                    advanced
                    min={0}
                    placeholder="400"
                    hint="u32，默认 400；0 = 不限（中文按 ≈1 token/字估算）。"
                  />
                  <NumberField
                    path="memory.recall_budget_ms"
                    label="召回耗时预算（毫秒）"
                    advanced
                    min={0}
                    placeholder="300"
                    hint="u64，默认 300；超时立即按「无记忆」继续，不拖慢首字延迟（TTFA）。"
                  />
                  <NumberField
                    path="memory.maintenance_interval"
                    label="维护间隔（轮）"
                    advanced
                    min={1}
                    placeholder="6"
                    hint="u32，默认 6；每 N 轮做一次维护（保留策略 + 隔离重试）。"
                  />
                  <NumberField
                    path="memory.quarantine_max_attempts"
                    label="抽取最大尝试次数"
                    advanced
                    min={1}
                    placeholder="3"
                    hint="u32，默认 3；超过该次数仍失败的抽取进入隔离区，不再无限重试（等 maintain 或人工处理）。"
                  />
                  <SelectField
                    path="memory.inject_position"
                    label="注入位置"
                    advanced
                    options={INJECT_POSITION_OPTIONS}
                    hint="after_history（默认）护住前缀缓存；before_history 离当前问题更近（recency 更强），但会破掉缓存。"
                  />
                </div>
              </SectionCard>

              <SectionCard
                title="抽取模型（OpenAI 兼容）"
                description="抽取专用模型路由；字段全空 = 复用 [llm]。api_key 是服务端 SECRET_PATHS：留空 = 保留原值，明文永不回传。"
              >
                <div className="divide-y divide-border">
                  <TextField
                    path="memory.extractor.api_base"
                    label="接口地址"
                    advanced
                    hint="OpenAI 兼容 base url；留空则用主 LLM 的配置。"
                  />
                  <SecretField
                    path="memory.extractor.api_key"
                    hasPath="memory.extractor.has_api_key"
                    label="API Key"
                    advanced
                  />
                  <TextField
                    path="memory.extractor.model"
                    label="模型名"
                    advanced
                    hint="抽取用模型名；留空则用主 LLM 的模型。"
                  />
                  <NumberField
                    path="memory.extractor.temperature"
                    label="随机性（temperature）"
                    advanced
                    integer={false}
                    placeholder="0.1"
                    hint="f32，默认 0.1：要稳定，不要发挥。唯一走小数通道的字段。"
                  />
                </div>
              </SectionCard>

              <SectionCard
                title="保留策略（[memory.retention]）"
                description="自动维护时对原文的清理规则；「只演练不删除」影响自动维护，下方「立即维护 / 演练保留策略」按钮会显式指定本次是否演练，与此字段独立。"
              >
                <div className="divide-y divide-border">
                  <SelectField
                    path="memory.retention.keep"
                    label="保留策略"
                    advanced
                    options={RETENTION_KEEP_OPTIONS}
                    hint="all = 全部保留（默认）；referenced = 只留被记忆引用的原文；recent = 最近 N 轮之外的未引用原文。"
                  />
                  <NumberField
                    path="memory.retention.recent_turns"
                    label="保留最近轮数"
                    advanced
                    min={0}
                    placeholder="0"
                    hint="u32，默认 0；keep = recent 时保留的轮数。"
                  />
                  <NumberField
                    path="memory.retention.retention_days"
                    label="保留天数"
                    advanced
                    min={0}
                    placeholder="0"
                    hint="u32，默认 0 = 不限；超过 N 天的未引用原文可删。"
                  />
                  <SwitchField
                    path="memory.retention.dry_run"
                    label="只演练不删除"
                    advanced
                    hint="开启后自动维护只统计、不真正删除。"
                  />
                </div>
              </SectionCard>

              {engine === "rag" ? (
                <SectionCard
                  title="文档 RAG（[memory.rag]，仅 engine = rag 时生效）"
                  description="扫描 docs_dir 下的 *.txt / *.md 建词法倒排索引，召回文档分块注入上下文；零 LLM 零外网。改动分块参数后需要「重建索引」。"
                >
                  <div className="space-y-3">
                    <div className="divide-y divide-border">
                      <TextField
                        path="memory.rag.docs_dir"
                        label="文档目录"
                        advanced
                        placeholder="/data/rag-docs"
                        hint="目录不存在 / 为空 = 零知识、零影响（对话完全正常，这是正常态不是降级）；容器部署请放在挂载卷内。"
                      />
                      <NumberField
                        path="memory.rag.chunk_size"
                        label="分块长度（字符）"
                        advanced
                        min={1}
                        placeholder="512"
                        hint="u32，默认 512；按空行分段后超长段再按此滑窗。改动后需重建索引。"
                      />
                      <NumberField
                        path="memory.rag.chunk_overlap"
                        label="分块重叠（字符）"
                        advanced
                        min={0}
                        placeholder="64"
                        hint="u32，默认 64；相邻分块的重叠字符数（保住跨块边界的语义）。"
                      />
                      <NumberField
                        path="memory.rag.top_k"
                        label="每次召回分块数"
                        advanced
                        min={1}
                        placeholder="4"
                        hint="u32，默认 4；每次召回最多注入几个分块。"
                      />
                    </div>
                    <div className="flex flex-wrap items-center gap-2">
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        disabled={anyBusy}
                        onClick={handleReindex}
                      >
                        {reindexBusy ? (
                          <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
                        ) : null}
                        重建索引
                      </Button>
                      <span className="text-sm text-muted-foreground">
                        异步后台任务：立即返回只代表「已受理」，前端不做自动轮询。
                      </span>
                    </div>
                    {reindexMessage !== null ? (
                      <FormNotice tone="info">{reindexMessage}</FormNotice>
                    ) : null}
                    {reindexError !== null ? (
                      <FormNotice tone="danger">重建索引失败：{reindexError}</FormNotice>
                    ) : null}
                  </div>
                </SectionCard>
              ) : null}
            </>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
