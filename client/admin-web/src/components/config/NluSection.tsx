/**
 * NLU 配置段主组件（W7 / Config C4）。
 *
 * 结构：标量字段（8 个）+ 技能/槽位表单编辑器（整段写回，见编辑器文件头）+
 * 模型全生命周期（构建 → 状态轮询 → 候选测试 → 显式采用）+ 只读保存态目录。
 *
 * 口径依据（全部已对源码核过）：
 * - 标量字段权威 `plugins/registry/fields_nlu.rs:5-46`；**不依赖 schema 的 nlu
 *   capability**（pure-base 构建下 `capabilities` 没有 nlu），但顶层
 *   `advanced_fields` 两种构建都全量下发，`isAdvancedField` 可用。
 * - 不暴露：`nlu.model_threshold` / `nlu.model_margin`（旧兼容字段，
 *   `plugins/nlu/mod.rs:44-49`）、`nlu.bert_label_map`（HashMap 无原语）。
 * - `nlu.bert_max_len` 不在 `advanced_fields`，不标 advanced；5 个 bert 字段
 *   （model/vocab/labels/threshold/num_threads）来自 `ADVANCED_KEYS`。
 * - `SectionHotHint sectionId="nlu"`：`APP_SECTION_HOT` 无 nlu，返回 null，
 *   不自造段级档位；保存响应 hot（字段级 next_session）由 SaveBar 正常显示。
 * - 简洁模式下 intents/slots 编辑器整块用 `FieldRow` advanced 门控折叠；
 *   编辑产生的 `nlu.intents` / `nlu.slots` dirty 路径命中 `ADVANCED_EXTRA_KEYS`，
 *   简洁模式二次确认由 `ConfigPage` 既有机制自动覆盖，这里不手写确认。
 * - 模型生命周期对齐旧端 `ConfigNluModelState.kt`：构建只读保存态、成功不自动
 *   启用、候选测试不写配置、采用 = 把 expected_config 的 bert 四字段写进草稿
 *   再由用户保存（无 enable 端点）。
 * - 底座名「minirbt-h288」是纯展示常量（`ConfigNluModelState.kt:48`），不参与
 *   任何请求；`BuildRequest` 带 `deny_unknown_fields`，绝不传 `base_model`。
 * - 不接 `/api/nlu/model/build/log` 的 WS：首帧是 `type:"state"` 非 backlog、
 *   终态后服务端主动 Close 且不可重连、还需 `?token=`——本期不做，用轮询。
 */

import { useCallback, useEffect, useRef, useState, type ReactElement } from "react";
import { useRequest } from "ahooks";
import { LoaderCircle, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { FieldRow } from "@/components/config/FieldRow";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { SwitchField } from "@/components/form/SwitchField";
import { TextField } from "@/components/form/TextField";
import { NumberField } from "@/components/form/NumberField";
import { FormNotice } from "@/components/form/FormNotice";
import { NluIntentsEditor } from "@/components/config/NluIntentsEditor";
import { NluSlotsEditor } from "@/components/config/NluSlotsEditor";
import { UnavailableNotice } from "@/components/feedback/UnavailableNotice";
import { useAdvancedMode } from "@/context/AdvancedModeContext";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import type { JsonObject } from "@/lib/configApi";
import {
  getNluBuildHistory,
  getNluBuildStatus,
  getSkillCatalog,
  isNluUnavailable,
  NLU_DOMAINS,
  nluErrorMessage,
  startNluBuild,
  testNluModel,
  type NluBuildHistory,
  type NluBuildStatus,
  type SkillCatalog,
} from "@/lib/nluApi";

/** 轮询参数：5s 间隔、360 次（30 分钟）上限，对齐旧端 `ConfigNluModelState.kt:140-164`。 */
const POLL_INTERVAL_MS = 5_000;
const POLL_MAX_ATTEMPTS = 360;

/** 底座展示常量：纯展示、不参与任何请求（`ConfigNluModelState.kt:42-48`）。 */
const BASE_MODEL_LABEL = "minirbt-h288";

/** 候选模型摘要（从 status.artifact 解析，判据对齐旧端 `parseCandidate:225-240`）。 */
interface NluCandidate {
  buildId: string;
  sourceRevision: string;
  sha256: string;
  /** artifact.base_model 只读记录；空串 = 历史构建未记录，绝不猜成当前底座。 */
  baseModel: string;
  expectedConfig: JsonObject;
}

/** 判断是否为普通 JSON 对象（数组与 null 不算）。 */
function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * 从 status.artifact 解析有效候选：build_id / source_revision / model_sha256
 * 非空，且 expected_config 含 bert_enabled=true 与三个非空路径字段；缺失时返回
 * null（页面显示「构建已结束，但服务端没有完整的候选版本信息」，不能直接启用）。
 */
function parseCandidate(artifact: JsonObject | null): NluCandidate | null {
  if (artifact === null) {
    return null;
  }
  const buildId = typeof artifact["build_id"] === "string" ? artifact["build_id"] : "";
  const sourceRevision =
    typeof artifact["source_revision"] === "string" ? artifact["source_revision"] : "";
  const sha256 = typeof artifact["model_sha256"] === "string" ? artifact["model_sha256"] : "";
  const baseModel = typeof artifact["base_model"] === "string" ? artifact["base_model"] : "";
  const expectedConfig = isJsonObject(artifact["expected_config"])
    ? artifact["expected_config"]
    : null;
  if (
    buildId === "" ||
    sourceRevision === "" ||
    sha256 === "" ||
    expectedConfig === null ||
    expectedConfig["bert_enabled"] !== true ||
    typeof expectedConfig["bert_model"] !== "string" ||
    expectedConfig["bert_model"] === "" ||
    typeof expectedConfig["bert_vocab"] !== "string" ||
    expectedConfig["bert_vocab"] === "" ||
    typeof expectedConfig["bert_labels"] !== "string" ||
    expectedConfig["bert_labels"] === ""
  ) {
    return null;
  }
  return { buildId, sourceRevision, sha256, baseModel, expectedConfig };
}

/** 测试结果与输入的绑定键：输入在响应回来前变化则结果不参与 verified（旧端 :222-223）。 */
function testKey(candidate: NluCandidate, text: string, expected: string): string {
  return JSON.stringify([
    candidate.buildId,
    candidate.sourceRevision,
    candidate.sha256,
    text,
    expected,
  ]);
}

/** integrity 四态的展示文案与配色。 */
function integrityBadge(integrity: string): { label: string; className: string } {
  switch (integrity) {
    case "ok":
      return { label: "ok", className: "border-emerald-600/40 bg-emerald-50 text-emerald-800" };
    case "unverified":
      return { label: "unverified", className: "border-amber-600/40 bg-amber-50 text-amber-800" };
    case "incomplete":
      return { label: "incomplete", className: "border-amber-600/40 bg-amber-50 text-amber-800" };
    case "corrupt":
      return {
        label: "corrupt",
        className: "border-destructive/40 bg-destructive/5 text-destructive",
      };
    default:
      return { label: integrity === "" ? "未知" : integrity, className: "" };
  }
}

export function NluSection(): ReactElement {
  const { advancedMode } = useAdvancedMode();
  const {
    revision,
    dirtyCount,
    saving,
    conflict,
    loading: draftLoading,
    setField,
    lastResult,
  } = useConfigDraft();
  const dirty = dirtyCount > 0;

  // ============================================================
  // 构建状态 + 轮询（5s × 360；useRequest pollingInterval，卸载/过期响应必弃）
  // ============================================================
  const [status, setStatus] = useState<NluBuildStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [polling, setPolling] = useState(false);
  const [pollMessage, setPollMessage] = useState<string | null>(null);
  /** 继续轮询的步进计数；每次「running 且未超次」的响应 +1（对齐旧 attemptsRef 口径）。 */
  const attemptsRef = useRef(0);

  const statusRequest = useRequest(getNluBuildStatus, {
    manual: true,
    pollingInterval: POLL_INTERVAL_MS,
    // 旧版 setTimeout 轮询不感知页面可见性，这里保持后台继续轮询。
    pollingWhenHidden: true,
    // 一次失败即停（对齐旧版 catch 不续轮询）。
    pollingErrorRetryCount: 0,
    onSuccess: (next: NluBuildStatus) => {
      setStatus(next);
      setStatusError(null);
      if (next.phase === "running" && attemptsRef.current < POLL_MAX_ATTEMPTS) {
        attemptsRef.current += 1;
        setPolling(true);
      } else {
        // cancel() 会递增 Fetch.count：本响应的 polling onFinally 不再调度下一次刷新。
        cancelStatusFetch();
        setPolling(false);
        if (next.phase === "running") {
          setPollMessage("已停止等待，服务器训练可能仍在继续。请稍后手动刷新状态。");
        }
      }
    },
    onError: (error: unknown) => {
      setStatusError(nluErrorMessage(error));
      cancelStatusFetch();
      setPolling(false);
    },
  });
  const { run: runStatusFetch, cancel: cancelStatusFetch, loading: statusLoading } = statusRequest;

  /** 手动刷新 / 构建启动后重挂轮询：attempts 归零（run 内部会先停掉待触发的轮询）。 */
  const refreshStatus = useCallback((): void => {
    attemptsRef.current = 0;
    setPollMessage(null);
    runStatusFetch();
  }, [runStatusFetch]);

  useEffect(() => {
    // 首次挂载读一次状态（可在页面刷新后重新挂上进行中的构建，对齐旧端 refresh）。
    refreshStatus();
    // ahooks 内置 useUnmount(() => cancel())；此处显式 cleanup 是对齐旧版「卸载即清
    // 轮询 timer」时序的冗余保险（cancel 同时清 polling 插件的待触发 timer）。
    return () => {
      cancelStatusFetch();
    };
  }, [refreshStatus, cancelStatusFetch]);

  const candidate = status === null ? null : parseCandidate(status.artifact);

  // ============================================================
  // 启动构建（门禁对齐旧端 ConfigNluModelState.kt:107-108）
  // ============================================================
  const [startError, setStartError] = useState<string | null>(null);
  const [startBusy, setStartBusy] = useState(false);

  const canStartBuild =
    !dirty &&
    !saving &&
    !conflict &&
    revision !== null &&
    revision !== "" &&
    !polling &&
    !startBusy &&
    !statusLoading;

  const handleStartBuild = async (): Promise<void> => {
    if (!canStartBuild || revision === null) {
      return;
    }
    setStartBusy(true);
    setStartError(null);
    try {
      await startNluBuild(revision);
      // 受理成功：立刻重挂轮询（fetchStatus 内部按 running 继续 5s 步进）。
      refreshStatus();
    } catch (error: unknown) {
      setStartError(nluErrorMessage(error));
    } finally {
      setStartBusy(false);
    }
  };

  // ============================================================
  // 候选测试（不写任何配置；60s 本地超时在 nluApi 层）
  // ============================================================
  const [testText, setTestText] = useState("");
  const [testExpected, setTestExpected] = useState("");
  const [testBusy, setTestBusy] = useState(false);
  const [testMessage, setTestMessage] = useState<string | null>(null);
  const [testVerifiedKey, setTestVerifiedKey] = useState<string | null>(null);
  const [adoptMessage, setAdoptMessage] = useState<string | null>(null);

  const testBusyOrPolling = testBusy || polling || startBusy;
  const canTest =
    candidate !== null &&
    testText.trim() !== "" &&
    testExpected.trim() !== "" &&
    !testBusyOrPolling;

  const handleTest = async (): Promise<void> => {
    if (candidate === null || !canTest) {
      return;
    }
    const key = testKey(candidate, testText.trim(), testExpected.trim());
    setTestBusy(true);
    setTestMessage("正在直接运行候选 ONNX 模型；不执行技能、不改变全局开关…");
    try {
      const result = await testNluModel({
        buildId: candidate.buildId,
        text: testText.trim(),
        expectedIntent: testExpected.trim(),
      });
      // 一致性强校验（旧端 ConfigNluModelState.kt:188-192 同款口径）。
      const valid =
        result.buildId === candidate.buildId &&
        result.modelSha256 === candidate.sha256 &&
        result.sourceRevision === candidate.sourceRevision &&
        result.expectedIntent === testExpected.trim();
      const unchanged = key === testKey(candidate, testText.trim(), testExpected.trim());
      const label =
        result.intent !== "" ? result.intent : result.rawLabel !== "" ? result.rawLabel : "未知";
      const verdict = valid
        ? `预测：${label}；置信度 ${result.confidence}；阈值 ${result.threshold}；` +
          (result.accepted ? "达到接受条件" : "拒识或未达阈值") +
          "；" +
          (result.expectedMatch === true
            ? "与预期一致"
            : result.expectedMatch === false
              ? "与预期不一致"
              : "无预期比对") +
          `。分词 ${result.tokenizeMs} ms，推理 ${result.inferMs} ms。未执行技能。`
        : "候选版本或推理证据不一致，不能采用。";
      setTestMessage(verdict);
      setTestVerifiedKey(
        valid && unchanged && result.accepted && result.expectedMatch === true ? key : null,
      );
    } catch (error: unknown) {
      setTestMessage(`候选测试失败：${nluErrorMessage(error)}`);
      setTestVerifiedKey(null);
    } finally {
      setTestBusy(false);
    }
  };

  const verified =
    candidate !== null &&
    testVerifiedKey !== null &&
    testVerifiedKey === testKey(candidate, testText.trim(), testExpected.trim());

  // ============================================================
  // 显式采用（adopt）：写草稿四字段，由用户保存；无 enable 端点
  // ============================================================
  const canAdopt =
    verified &&
    candidate !== null &&
    candidate.sourceRevision === revision &&
    !dirty &&
    !saving &&
    !conflict &&
    !polling &&
    !testBusy &&
    !startBusy;

  const handleAdopt = (): void => {
    if (!canAdopt || candidate === null) {
      return;
    }
    const cfg = candidate.expectedConfig;
    setField("nlu.bert_enabled", cfg["bert_enabled"] === true);
    setField("nlu.bert_model", typeof cfg["bert_model"] === "string" ? cfg["bert_model"] : "");
    setField("nlu.bert_vocab", typeof cfg["bert_vocab"] === "string" ? cfg["bert_vocab"] : "");
    setField("nlu.bert_labels", typeof cfg["bert_labels"] === "string" ? cfg["bert_labels"] : "");
    setAdoptMessage(
      "已将候选写入当前草稿（bert_enabled / bert_model / bert_vocab / bert_labels 四字段），请检查后点保存生效；构建永不自动启用。采用候选并保存启用后，首启自举可能改配置版本，保存报 409 请重新加载。",
    );
  };

  // ============================================================
  // 构建历史
  // ============================================================
  const [history, setHistory] = useState<NluBuildHistory | null>(null);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [historyLoading, setHistoryLoading] = useState(false);

  const loadHistory = useCallback(async (): Promise<void> => {
    setHistoryLoading(true);
    try {
      setHistory(await getNluBuildHistory());
      setHistoryError(null);
    } catch (error: unknown) {
      setHistoryError(nluErrorMessage(error));
    } finally {
      setHistoryLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadHistory();
  }, [loadHistory]);

  // 构建结束后自动刷新历史（从 running 转非 running 的一次性触发）。
  const prevPhaseRef = useRef<string | null>(null);
  useEffect(() => {
    const phase = status?.phase ?? null;
    if (prevPhaseRef.current === "running" && phase !== null && phase !== "running") {
      void loadHistory();
    }
    prevPhaseRef.current = phase;
  }, [status, loadHistory]);

  // ============================================================
  // 保存态技能目录（只读，fail-closed；保存成功后自动刷新）
  // ============================================================
  const [catalog, setCatalog] = useState<SkillCatalog | null>(null);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [catalogUnavailable, setCatalogUnavailable] = useState(false);
  const [catalogLoading, setCatalogLoading] = useState(true);

  const loadCatalog = useCallback(async (): Promise<void> => {
    setCatalogLoading(true);
    try {
      setCatalog(await getSkillCatalog());
      setCatalogError(null);
      setCatalogUnavailable(false);
    } catch (error: unknown) {
      setCatalog(null);
      setCatalogUnavailable(isNluUnavailable(error));
      setCatalogError(nluErrorMessage(error));
    } finally {
      setCatalogLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadCatalog();
  }, [loadCatalog]);

  useEffect(() => {
    // 保存成功（回读刷新 baseline）后目录快照可能过期，自动重拉。
    if (lastResult !== null) {
      void loadCatalog();
    }
  }, [lastResult, loadCatalog]);

  const statusBusy = polling || statusLoading;
  const statusLine = (() => {
    if (statusError !== null) {
      return `状态读取失败：${statusError}`;
    }
    if (status === null) {
      return draftLoading ? "正在读取训练状态…" : "尚未读到训练状态。";
    }
    switch (status.phase) {
      case "running":
        return `训练中；使用保存态版本 ${status.sourceRevision ?? "未知"}，尚未启用。`;
      case "success":
        return candidate === null
          ? "构建已结束，但服务端没有完整的候选版本信息。请更新服务端；不能直接启用。"
          : "候选模型已生成，尚未启用。请先独立测试新技能与拒识样例。";
      case "failed":
        return `训练失败：${status.error ?? "请查看日志"}`;
      default:
        return "暂无训练任务。先保存技能，再构建候选模型。";
    }
  })();

  return (
    <div className="space-y-4">
      {/* APP_SECTION_HOT 无 nlu，SectionHotHint 返回 null；不自造段级档位。 */}
      <SectionHotHint sectionId="nlu" />

      <SectionCard
        title="NLU · 意图与技能"
        description="技能只能由真实分类器选择；训练例句只用于生成语料，不是快捷触发指令。没有规则、启发式或 LLM 重新识别。"
      >
        <div className="divide-y divide-border">
          <SwitchField
            path="nlu.enabled"
            label="启用技能目录"
            hint="启用保存态技能目录；只有真实训练模型可选择意图。command 目录独立启停，但也必须通过模型。"
          />
          <SwitchField
            path="nlu.bert_enabled"
            label="启用模型意图分类"
            hint="唯一意图识别开关（默认 false）。关闭时只允许普通聊天，无技能工具；启用却加载或推理失败时明确错误，不回退规则或 LLM。"
          />
          <TextField
            path="nlu.bert_model"
            label="TinyBERT 模型路径"
            advanced
            placeholder="/data/models/nlu/model.onnx"
            hint="ONNX 模型文件，与词表及标签对应；bert_enabled=true 时必填且文件存在。"
          />
          <TextField
            path="nlu.bert_vocab"
            label="TinyBERT 词表路径"
            advanced
            placeholder="/data/models/nlu/tokenizer.json"
            hint="与模型配套的 tokenizer vocab.txt，启用模型时必填。"
          />
          <TextField
            path="nlu.bert_labels"
            label="TinyBERT 标签路径"
            advanced
            placeholder="/data/models/nlu/labels.txt"
            hint="每行一个标签，顺序必须与输出 logits 一致；other 是保留拒识标签。"
          />
          <NumberField
            path="nlu.bert_threshold"
            label="模型接受阈值"
            advanced
            integer={false}
            min={0}
            max={1}
            placeholder="0.75"
            hint="置信度达到阈值才接受同一技能；取值 0~1 的有限数，不能仅靠高阈值代替负例验收。"
          />
          <NumberField
            path="nlu.bert_num_threads"
            label="模型推理线程数"
            advanced
            min={1}
            placeholder="1"
            hint="ONNX Runtime 会话内线程数，默认 1 以减少与音频链路争用。"
          />
          <NumberField
            path="nlu.bert_max_len"
            label="模型最大序列长度"
            min={1}
            placeholder="64"
            hint="分词截断/填充长度，必须与导出模型一致。"
          />
        </div>
        <div className="mt-2">
          <FormNotice tone="info">
            旧兼容字段 model_threshold / model_margin 与 bert_label_map 不参与识别，界面不提供编辑。
          </FormNotice>
        </div>

        {/* 技能目录编辑：简洁模式整块折叠，给可行动提示。dirty 路径 nlu.intents /
            nlu.slots 命中 ADVANCED_EXTRA_KEYS，二次确认由 ConfigPage 既有机制覆盖。 */}
        <div className="mt-4 space-y-3">
          {!advancedMode ? (
            <FormNotice tone="info">
              技能目录编辑属高级操作，请切换到高级模式后编辑技能与槽位定义。
            </FormNotice>
          ) : null}
          <FieldRow
            label="技能目录编辑（高级）"
            hint="改名、增删技能与槽位都直接改写草稿中的 nlu.intents / nlu.slots（整段写回）。"
            advanced
          >
            <div className="space-y-4">
              <NluIntentsEditor />
              <NluSlotsEditor />
            </div>
          </FieldRow>
        </div>
      </SectionCard>

      <SectionCard
        title="模型构建与候选"
        description={`当前固定使用预训练底座 ${BASE_MODEL_LABEL}（纯展示；构建请求不带底座字段，服务端只有唯一底座）。`}
      >
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={!canStartBuild || draftLoading}
              onClick={() => void handleStartBuild()}
            >
              {startBusy ? (
                <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
              ) : null}
              开始构建
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={statusBusy || startBusy}
              onClick={refreshStatus}
            >
              <RefreshCw aria-hidden="true" className="size-4" />
              刷新状态
            </Button>
            {polling ? (
              <Badge variant="outline">
                轮询中（第 {attemptsRef.current}/{POLL_MAX_ATTEMPTS} 次，5s 间隔）
              </Badge>
            ) : null}
          </div>
          {!canStartBuild && revision !== null && dirty ? (
            <FormNotice tone="warning">
              构建只读取服务器已保存配置，不读取当前输入草稿：存在未保存改动时不能启动构建，请先保存或放弃改动。
            </FormNotice>
          ) : null}
          {!canStartBuild && conflict ? (
            <FormNotice tone="warning">
              当前处于保存冲突状态：请先「重新加载并放弃本地改动」再启动构建。
            </FormNotice>
          ) : null}
          {startError !== null ? (
            <FormNotice tone="danger">启动构建失败：{startError}</FormNotice>
          ) : null}
          <div className="flex flex-wrap items-center gap-2">
            {statusLoading ? (
              <LoaderCircle
                aria-hidden="true"
                className="size-4 animate-spin text-muted-foreground"
              />
            ) : null}
            <p className="text-sm font-medium">{statusLine}</p>
          </div>
          {pollMessage !== null ? <FormNotice tone="warning">{pollMessage}</FormNotice> : null}
          {status !== null && status.logTail !== "" ? (
            <div className="space-y-1">
              <pre className="max-h-64 overflow-auto whitespace-pre-wrap rounded-md bg-zinc-950 px-3 py-2 font-mono text-xs text-zinc-100">
                {status.logTail}
              </pre>
              <p className="text-xs text-muted-foreground">
                尾部日志（服务端截断 8192 字符），非全量增量；实时增量日志流本期不接入。
              </p>
            </div>
          ) : null}

          {candidate !== null ? (
            <div className="space-y-2 rounded-md border border-border px-3 py-2">
              <p className="text-sm font-medium">候选模型</p>
              <div className="grid gap-1 text-xs">
                <p>
                  <span className="font-medium">build_id：</span>
                  <span className="font-mono">{candidate.buildId}</span>
                </p>
                <p>
                  <span className="font-medium">保存态版本：</span>
                  <span className="font-mono">{candidate.sourceRevision}</span>
                </p>
                <p>
                  <span className="font-medium">model_sha256：</span>
                  <span className="font-mono break-all">{candidate.sha256}</span>
                </p>
                <p>
                  <span className="font-medium">训练底座：</span>
                  {candidate.baseModel === "" ? "未记录（历史构建）" : candidate.baseModel}
                </p>
                <p>
                  <span className="font-medium">启用后路径：</span>
                  <span className="font-mono break-all">
                    {typeof candidate.expectedConfig["bert_model"] === "string"
                      ? candidate.expectedConfig["bert_model"]
                      : ""}
                  </span>
                </p>
              </div>
              <div className="grid gap-1.5 pt-1">
                <p className="text-sm font-medium">候选测试</p>
                <Input
                  className="max-w-md"
                  value={testText}
                  placeholder="测试句子，如：十分钟后提醒我喝水"
                  onChange={(event) => setTestText(event.target.value)}
                />
                <Input
                  className="max-w-md font-mono"
                  value={testExpected}
                  placeholder="期望技能名（必填），如：reminder_countdown"
                  onChange={(event) => setTestExpected(event.target.value)}
                />
                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={!canTest}
                    onClick={() => void handleTest()}
                  >
                    {testBusy ? (
                      <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
                    ) : null}
                    运行候选测试
                  </Button>
                  {verified ? (
                    <Badge
                      className="border-emerald-600/40 bg-emerald-50 text-emerald-800"
                      variant="outline"
                    >
                      已验证（可用「采用候选」）
                    </Badge>
                  ) : null}
                </div>
                <FormNotice tone="info">
                  候选测试不改变全局开关，也不执行设备动作；只做意图分类，不做槽位抽取或技能执行。
                </FormNotice>
                {testMessage !== null ? (
                  <p
                    className={
                      testVerifiedKey !== null
                        ? "text-sm font-medium text-emerald-800"
                        : "text-sm font-medium"
                    }
                  >
                    {testMessage}
                  </p>
                ) : null}
                {testMessage !== null ? (
                  <FormNotice tone="warning">
                    一个正例达阈值不代表整体准确率、槽位抽取或设备动作已验收；请覆盖正例与拒识样例后再采用。
                  </FormNotice>
                ) : null}
              </div>
              <div className="pt-1">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={!canAdopt}
                  onClick={handleAdopt}
                >
                  采用候选（写入草稿，保存后生效）
                </Button>
                {!canAdopt && verified ? (
                  <p className="mt-1 text-xs text-muted-foreground">
                    采用前置：候选的保存态版本与当前配置一致、无未保存改动、无冲突且不在构建/测试中。
                  </p>
                ) : null}
                {adoptMessage !== null ? (
                  <div className="mt-2">
                    <FormNotice tone="info">{adoptMessage}</FormNotice>
                  </div>
                ) : null}
              </div>
            </div>
          ) : null}
        </div>
      </SectionCard>

      <SectionCard
        title="构建历史"
        description="按创建时间倒序（服务端已排序、截断上限）；integrity 四态：ok / unverified / incomplete / corrupt。"
      >
        <div className="space-y-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={historyLoading}
            onClick={() => void loadHistory()}
          >
            {historyLoading ? (
              <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
            ) : (
              <RefreshCw aria-hidden="true" className="size-4" />
            )}
            刷新历史
          </Button>
          {historyError !== null ? (
            <FormNotice tone="danger">读取构建历史失败：{historyError}</FormNotice>
          ) : null}
          {history !== null && history.builds.length === 0 ? (
            <p className="text-xs text-muted-foreground">从未构建过模型。</p>
          ) : null}
          {history !== null && history.builds.length > 0 ? (
            <div className="overflow-x-auto">
              <table className="w-full min-w-[720px] text-left text-xs">
                <thead>
                  <tr className="border-b border-border">
                    <th className="py-1.5 pr-3 font-medium">build_id</th>
                    <th className="py-1.5 pr-3 font-medium">created_at</th>
                    <th className="py-1.5 pr-3 font-medium">base_model</th>
                    <th className="py-1.5 pr-3 font-medium">labels</th>
                    <th className="py-1.5 pr-3 font-medium">integrity</th>
                    <th className="py-1.5 pr-3 font-medium">missing</th>
                    <th className="py-1.5 pr-3 font-medium">active</th>
                    <th className="py-1.5 font-medium">error</th>
                  </tr>
                </thead>
                <tbody>
                  {history.builds.map((row) => {
                    const badge = integrityBadge(row.integrity);
                    return (
                      <tr key={row.buildId} className="border-b border-border/60 align-top">
                        <td className="py-1.5 pr-3 font-mono">{row.buildId}</td>
                        <td className="py-1.5 pr-3">{row.createdAt ?? "未记录（历史构建）"}</td>
                        <td className="py-1.5 pr-3">{row.baseModel ?? "未记录（历史构建）"}</td>
                        <td className="py-1.5 pr-3">共 {row.labels.length} 个标签</td>
                        <td className="py-1.5 pr-3">
                          <Badge variant="outline" className={badge.className}>
                            {badge.label}
                          </Badge>
                        </td>
                        <td className="py-1.5 pr-3">
                          {row.missing.length > 0 ? `缺 ${row.missing.length} 个文件` : "—"}
                        </td>
                        <td className="py-1.5 pr-3">{row.active ? "是" : "否"}</td>
                        <td className="py-1.5 max-w-[220px] break-all">{row.error ?? "—"}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          ) : null}
        </div>
      </SectionCard>

      <SectionCard
        title="保存态技能目录"
        description="GET /api/skills/catalog 的只读快照（与上方编辑器并存：编辑结果保存并构建后才进入目录）；按控制域分组。"
      >
        <div className="space-y-3">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={catalogLoading}
            onClick={() => void loadCatalog()}
          >
            {catalogLoading ? (
              <LoaderCircle aria-hidden="true" className="size-4 animate-spin" />
            ) : (
              <RefreshCw aria-hidden="true" className="size-4" />
            )}
            刷新目录
          </Button>
          {catalogUnavailable ? (
            <UnavailableNotice
              title="技能目录能力未启用"
              description={
                catalogError ??
                "服务端未挂载 /api/skills/catalog（通常是构建未启用对应能力）；该状态下不能查看保存态目录，配置编辑仍可用。"
              }
            />
          ) : catalogError !== null ? (
            <FormNotice tone="danger">读取技能目录失败：{catalogError}</FormNotice>
          ) : null}
          {catalog !== null ? (
            <div className="space-y-3">
              <p className="text-xs text-muted-foreground">
                识别口径 {catalog.recognition === "" ? "未知" : catalog.recognition}；目录版本{" "}
                <span className="font-mono">
                  {catalog.revision === "" ? "未知" : catalog.revision}
                </span>
                ；共 {catalog.intents.length} 个技能。
              </p>
              {(["server", "device", "iot"] as const).map((domain) => {
                const intents = catalog.intents.filter((item) => item.domain === domain);
                const label =
                  domain === "server"
                    ? "服务器域（服务端执行）"
                    : domain === "device"
                      ? "设备域（固件执行）"
                      : "IoT 域（第三方设备 · tuya）";
                return (
                  <div key={domain} className="space-y-1.5">
                    <p className="text-sm font-medium">
                      {label}（{intents.length} 条）
                    </p>
                    {intents.length === 0 ? (
                      <p className="text-xs text-muted-foreground">无。</p>
                    ) : (
                      <ul className="space-y-1">
                        {intents.map((item, index) => (
                          <li
                            key={`${item.name}:${index}`}
                            className="flex flex-wrap items-center gap-2 text-xs"
                          >
                            <span className="font-mono font-medium">{item.name}</span>
                            <Badge variant="outline">
                              {item.source === "" ? "未知来源" : item.source}
                            </Badge>
                            <span className="text-muted-foreground">
                              例句 {item.templates.length} 条
                            </span>
                            {item.capabilities.length > 0 ? (
                              <span className="text-muted-foreground">
                                能力：{item.capabilities.join("、")}
                              </span>
                            ) : null}
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>
                );
              })}
              {catalog.intents.some(
                (item) => !(NLU_DOMAINS as readonly string[]).includes(item.domain),
              ) ? (
                <p className="text-xs font-medium text-amber-900">
                  目录中存在未知控制域的技能条目，已不在上述分组中显示；以服务端为准，不要在界面改写。
                </p>
              ) : null}
            </div>
          ) : null}
        </div>
      </SectionCard>
    </div>
  );
}
