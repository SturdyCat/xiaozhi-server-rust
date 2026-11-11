/**
 * 硬件测试台（仅 Tauri 环境挂载，nav `tauriOnly: true`）：五 tab 工作区。
 *
 * 结构对齐旧端 `TestBenchPage.kt:67-81` tab 常量与 `:144-162` 页头：
 * 连接 / ASR 识别 / TTS 合成 / LLM 对话 / NLU 意图（本波 W14a 落地连接 + LLM + NLU
 * 三个纯文本 tab；ASR / TTS 依赖麦克风、Opus 编解码与播放器，需 Tauri 桌面硬件桥接
 * ——用户拍板硬件功能用 Tauri app 实现（W14b 架构深化），Web 不支持的部分无需入口，
 * 两个 tab 保留结构给诚实说明卡，0 硬件动作）。
 *
 * 流程口径（逐字对齐旧端）：
 * - LLM：发 `llm_test` → 服务端按磁盘最新 `[llm]` 配置直调 LLM → 回包带回复与服务端
 *   耗时（改配置后无需重启即可验证；`TestBenchLlmNluPage.kt:44`）；
 * - NLU：发 `nlu_test` → 服务端按磁盘最新 `[nlu]` 配置构建 TinyBERT 直推 → 回包带
 *   意图、置信度与分词/推理耗时（`:105-106`，未配置时回错误原因）。
 *
 * 与旧端的已登记差异：右栏配置卡不嵌入（旧端与配置页同源，admin-web 的 server /
 * LLM / NLU 配置在「配置管理」页对应段完成）；桌面壳能力检测卡随重写移除（旧端无
 * 此卡；`platform.ts` 的 app_capabilities 通道保留，W14b 需要时可回挂）。
 * WS 帧协议见 `lib/testbenchApi.ts` 头注释。超时：LLM 60s / NLU 10s（对齐
 * `TestBenchState.kt:346/:431`），超时文案逐字；断线不自动重连，手动点「连接」。
 */

import { useEffect, useRef, useState, type ReactElement } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { copyText } from "@/lib/clipboard";
import {
  TestbenchSocket,
  buildTestbenchWsUrl,
  probeTestbenchHandshakeDetail,
  resolveTestbenchToken,
  type LlmTestWire,
  type NluTestWire,
} from "@/lib/testbenchApi";

/** tab id 常量（对齐 TestBenchPage.kt:67-71）。 */
type BenchTab = "connect" | "asr" | "tts" | "llm" | "nlu";

const BENCH_TABS: readonly { id: BenchTab; label: string }[] = [
  { id: "connect", label: "连接" },
  { id: "asr", label: "ASR 识别" },
  { id: "tts", label: "TTS 合成" },
  { id: "llm", label: "LLM 对话" },
  { id: "nlu", label: "NLU 意图" },
];

// tab 选中态的会话级缓存（对齐旧端 benchTabs 按 Pager 缓存：切走再切回不丢选中 tab）。
let lastBenchTab: BenchTab = "connect";

/** 连接状态 → 中文徽标文案（AdminNav.kt:110-115 connectionLabel 逐字）。 */
function connectionLabel(state: "idle" | "connecting" | "connected" | "error"): string {
  switch (state) {
    case "connected":
      return "已连接";
    case "connecting":
      return "连接中";
    case "error":
      return "连接错误";
    default:
      return "未连接";
  }
}

/** 连接状态 → 徽标 variant。 */
function connectionBadgeVariant(
  state: "idle" | "connecting" | "connected" | "error",
): "default" | "secondary" | "destructive" | "outline" {
  switch (state) {
    case "connected":
      return "default";
    case "connecting":
      return "secondary";
    case "error":
      return "destructive";
    default:
      return "outline";
  }
}

/** LLM 结果块（对齐旧端 BenchResultBlock：正文 + 耗时明细 + 复制）。 */
interface LlmBenchState {
  prompt: string;
  busy: boolean;
  ok: boolean;
  reply: string;
  elapsedMs: number;
  copyHint: string;
}

/** NLU 结果块（对齐旧端 TestBenchState nlu* 字段组）。 */
interface NluBenchState {
  input: string;
  busy: boolean;
  ok: boolean;
  intent: string;
  confidence: number | null;
  tokenizeMs: number;
  inferMs: number;
  text: string;
  copyHint: string;
}

const LLM_TIMEOUT_MS = 60_000; // TestBenchState.kt:346
const NLU_TIMEOUT_MS = 10_000; // TestBenchState.kt:431

/** 灰字说明行（对齐旧端 HintLine 零件：结果块的占位/耗时明细）。 */
function BenchHint({ text }: { text: string }): ReactElement {
  return <p className="text-xs text-muted-foreground">{text}</p>;
}

export function TestbenchPage(): ReactElement {
  const [tab, setTab] = useState<BenchTab>(lastBenchTab);
  const [connectionState, setConnectionState] = useState<
    "idle" | "connecting" | "connected" | "error"
  >("idle");
  const [tokenInput, setTokenInput] = useState("");
  const [statusMsg, setStatusMsg] = useState("");
  const [llm, setLlm] = useState<LlmBenchState>({
    prompt: "你好，请用一句话介绍你自己",
    busy: false,
    ok: false,
    reply: "",
    elapsedMs: 0,
    copyHint: "",
  });
  const [nlu, setNlu] = useState<NluBenchState>({
    input: "把客厅的灯打开",
    busy: false,
    ok: false,
    intent: "",
    confidence: null,
    tokenizeMs: 0,
    inferMs: 0,
    text: "",
    copyHint: "",
  });

  const socketRef = useRef<TestbenchSocket | null>(null);
  const llmTimerRef = useRef<number | null>(null);
  const nluTimerRef = useRef<number | null>(null);

  // tab 切换写会话级缓存。
  const handleTabChange = (value: string): void => {
    if (BENCH_TABS.some((item) => item.id === value)) {
      lastBenchTab = value as BenchTab;
      setTab(lastBenchTab);
    }
  };

  // 卸载时关连接、清超时定时器（不自动重连）。
  useEffect(() => {
    return () => {
      socketRef.current?.close();
      socketRef.current = null;
      if (llmTimerRef.current !== null) {
        window.clearTimeout(llmTimerRef.current);
      }
      if (nluTimerRef.current !== null) {
        window.clearTimeout(nluTimerRef.current);
      }
    };
  }, []);

  /** 三态连接按钮：未连=连接、连接中=菊花拦截、已连=断开。 */
  const handleConnectToggle = (): void => {
    if (connectionState === "connecting") {
      return;
    }
    if (connectionState === "connected") {
      socketRef.current?.close();
      socketRef.current = null;
      setConnectionState("idle");
      setStatusMsg("已断开");
      return;
    }

    const token = resolveTestbenchToken(tokenInput);
    const url = buildTestbenchWsUrl(token);
    if (url === null) {
      setConnectionState("error");
      setStatusMsg("连接失败: 无法推导 WS 地址（当前环境无窗口）");
      return;
    }
    setConnectionState("connecting");
    setStatusMsg("连接中…");
    const socket = new TestbenchSocket(url, {
      onOpen: () => {
        setConnectionState("connected");
        setStatusMsg("已连接");
      },
      onHandshakeFailure: () => {
        setConnectionState("error");
        setStatusMsg("连接失败: WebSocket 握手失败");
        // dev 模式下握手被 401/503 拦：借 dev guard 的 JSON 原文透出真实原因（见探针注释）。
        void probeTestbenchHandshakeDetail(token).then((detail) => {
          if (detail !== null) {
            setStatusMsg(`连接失败: ${detail}`);
          }
        });
      },
      onUnexpectedClose: (code) => {
        setConnectionState("error");
        setStatusMsg(`连接失败: 连接已中断（close code ${code}）`);
      },
      onLlmTestResult: (result: LlmTestWire) => {
        if (llmTimerRef.current !== null) {
          window.clearTimeout(llmTimerRef.current);
          llmTimerRef.current = null;
        }
        const ok = result.state === "ok";
        const elapsedMs = result.elapsedMs ?? 0;
        // state=ok 但正文为空是合法回包（模型只回工具调用等）——单独提示，不冒充成功正文。
        const reply =
          (result.text ?? "").trim() === "" && ok ? "（LLM 返回了空回复）" : (result.text ?? "");
        setLlm((previous) => ({ ...previous, busy: false, ok, reply, elapsedMs }));
        setStatusMsg(ok ? `LLM 正常（服务端耗时 ${elapsedMs}ms）` : `LLM 测试失败：${reply}`);
      },
      onNluTestResult: (result: NluTestWire) => {
        if (nluTimerRef.current !== null) {
          window.clearTimeout(nluTimerRef.current);
          nluTimerRef.current = null;
        }
        if (result.state === "ok") {
          const intent = result.intent ?? "";
          const reason = result.text ?? "";
          setNlu((previous) => ({
            ...previous,
            busy: false,
            ok: true,
            intent,
            confidence: result.confidence,
            tokenizeMs: result.tokenizeMs ?? 0,
            inferMs: result.inferMs ?? 0,
            text: reason,
          }));
          setStatusMsg(
            intent === ""
              ? `未命中意图：${reason}`
              : `命中意图 ${intent}（分词 ${result.tokenizeMs ?? 0}ms / 推理 ${result.inferMs ?? 0}ms）`,
          );
        } else {
          const reason =
            (result.text ?? "").trim() === "" ? "（服务端未返回原因）" : (result.text ?? "");
          setNlu((previous) => ({
            ...previous,
            busy: false,
            ok: false,
            intent: "",
            confidence: null,
            tokenizeMs: 0,
            inferMs: 0,
            text: reason,
          }));
          setStatusMsg(`NLU 测试失败：${reason}`);
        }
      },
    });
    socketRef.current = socket;
    socket.open();
  };

  /** LLM 测试：前置校验 → 发帧 → 超时兜底（文案逐字）。 */
  const handleTestLlm = (): void => {
    if (llm.busy) {
      return;
    }
    if (connectionState !== "connected") {
      setStatusMsg("请先在「连接」卡片建立连接");
      return;
    }
    if (llm.prompt.trim() === "") {
      setStatusMsg("请输入测试提示词");
      return;
    }
    setLlm((previous) => ({
      ...previous,
      busy: true,
      ok: false,
      reply: "",
      elapsedMs: 0,
      copyHint: "",
    }));
    setStatusMsg("LLM 测试中…");
    const sent = socketRef.current?.sendLlmTest(llm.prompt) ?? false;
    if (!sent) {
      setLlm((previous) => ({ ...previous, busy: false }));
      setConnectionState("error");
      setStatusMsg("请先在「连接」卡片建立连接");
      return;
    }
    llmTimerRef.current = window.setTimeout(() => {
      llmTimerRef.current = null;
      setLlm((previous) => ({ ...previous, busy: false }));
      setStatusMsg("LLM 测试超时：请检查 api_base/api_key 或服务器日志");
    }, LLM_TIMEOUT_MS);
  };

  /** NLU 测试：前置校验 → 发帧 → 超时兜底（文案逐字）。 */
  const handleTestNlu = (): void => {
    if (nlu.busy) {
      return;
    }
    if (connectionState !== "connected") {
      setStatusMsg("请先在「连接」卡片建立连接");
      return;
    }
    if (nlu.input.trim() === "") {
      setStatusMsg("请输入测试文本");
      return;
    }
    setNlu((previous) => ({
      ...previous,
      busy: true,
      ok: false,
      intent: "",
      confidence: null,
      tokenizeMs: 0,
      inferMs: 0,
      text: "",
      copyHint: "",
    }));
    setStatusMsg("NLU 测试中…");
    const sent = socketRef.current?.sendNluTest(nlu.input) ?? false;
    if (!sent) {
      setNlu((previous) => ({ ...previous, busy: false }));
      setConnectionState("error");
      setStatusMsg("请先在「连接」卡片建立连接");
      return;
    }
    nluTimerRef.current = window.setTimeout(() => {
      nluTimerRef.current = null;
      setNlu((previous) => ({ ...previous, busy: false }));
      setStatusMsg("NLU 测试超时：请检查连接或服务器日志");
    }, NLU_TIMEOUT_MS);
  };

  /** 结果整段文本（NLU 详情面板与复制共用，对齐旧端 nluResultText）。 */
  const nluResultText = (): string => {
    if (!nlu.ok) {
      return nlu.text;
    }
    const headLines: string[] = [`意图：${nlu.intent === "" ? "（未命中）" : nlu.intent}`];
    if (nlu.confidence !== null) {
      headLines.push(`置信度：${Math.round(nlu.confidence * 1000) / 1000}`);
    }
    headLines.push(`耗时：分词 ${nlu.tokenizeMs}ms / 推理 ${nlu.inferMs}ms`);
    const head = headLines.join("\n");
    return nlu.text === "" ? head : `${head}\n${nlu.text}`;
  };

  /** 复制（三级降级 copyText；卡片内 hint 反馈，对齐旧端 finishCopy 语义）。 */
  const handleCopy = (text: string, setHint: (hint: string) => void): void => {
    const trimmed = text.trim();
    if (trimmed === "") {
      setHint("没有可复制的内容");
      return;
    }
    void copyText(trimmed).then((ok) => {
      setHint(ok ? `已复制 ${trimmed.length} 字` : "复制失败");
    });
  };

  const wsUrl = buildTestbenchWsUrl(null);

  return (
    <div className="mx-auto w-full max-w-4xl space-y-6">
      {/* 页头（对齐旧端 BenchHeader：标题 + statusMsg 状态行，空时给固定说明） */}
      <div className="space-y-1">
        <h2 className="text-2xl font-semibold tracking-tight">测试台</h2>
        <p className="text-sm text-muted-foreground">
          {statusMsg === ""
            ? "连接服务端后测试 LLM 对话 / NLU 意图；录音识别与 TTS 合成依赖硬件桥接，延后接入。"
            : statusMsg}
        </p>
      </div>
      <Separator />
      <Tabs value={tab} onValueChange={handleTabChange}>
        <TabsList className="w-full justify-start overflow-x-auto">
          {BENCH_TABS.map((item) => (
            <TabsTrigger key={item.id} value={item.id}>
              {item.label}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>

      {tab === "connect" ? (
        <Card>
          <CardHeader>
            <CardTitle>连接</CardTitle>
            <CardDescription>
              服务器地址与 API 同源推导；仅当服务端设置 `[server].expected_token` 时需要填写
              token（浏览器 WebSocket 走 `?token=` 查询参数）。
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            <p className="break-all font-mono text-xs text-muted-foreground">
              服务器：{wsUrl ?? "（当前环境无法推导 WS 地址）"}
            </p>
            <div className="space-y-1.5">
              <Label htmlFor="bench-token">token（可选）</Label>
              <Input
                id="bench-token"
                type="password"
                value={tokenInput}
                onChange={(event) => setTokenInput(event.target.value)}
                placeholder="留空则沿用页面 URL 携带的令牌"
              />
            </div>
            <div className="flex items-center gap-3">
              <Button
                onClick={handleConnectToggle}
                disabled={connectionState === "connecting"}
                variant={connectionState === "connected" ? "destructive" : "default"}
              >
                {connectionState === "connecting"
                  ? "连接中…"
                  : connectionState === "connected"
                    ? "断开"
                    : "连接"}
              </Button>
              <Badge variant={connectionBadgeVariant(connectionState)}>
                {connectionLabel(connectionState)}
              </Badge>
            </div>
          </CardContent>
        </Card>
      ) : null}

      {tab === "asr" ? (
        <Card>
          <CardHeader>
            <CardTitle>ASR 识别</CardTitle>
            <CardDescription>录音识别依赖本机硬件能力，本版本不提供入口。</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 text-sm text-muted-foreground">
            <p>
              录音识别与合成播放依赖本机麦克风、Opus 编解码与播放器，需 Tauri
              桌面环境的硬件桥接（架构深化中，延后接入）；当前版本不提供入口。
            </p>
            <div className="flex flex-wrap gap-2">
              <Badge variant="outline">接口可用 ≠ 真机通过</Badge>
              <Badge variant="outline">
                测试不证明原生模型可加载、真实 TTS 音色可用、设备收到数据或扬声器播放
              </Badge>
            </div>
          </CardContent>
        </Card>
      ) : null}

      {tab === "tts" ? (
        <Card>
          <CardHeader>
            <CardTitle>TTS 合成</CardTitle>
            <CardDescription>合成播放依赖本机硬件能力，本版本不提供入口。</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 text-sm text-muted-foreground">
            <p>
              录音识别与合成播放依赖本机麦克风、Opus 编解码与播放器，需 Tauri
              桌面环境的硬件桥接（架构深化中，延后接入）；当前版本不提供入口。
            </p>
            <div className="flex flex-wrap gap-2">
              <Badge variant="outline">接口可用 ≠ 真机通过</Badge>
              <Badge variant="outline">
                测试不证明原生模型可加载、真实 TTS 音色可用、设备收到数据或扬声器播放
              </Badge>
            </div>
          </CardContent>
        </Card>
      ) : null}

      {tab === "llm" ? (
        <Card>
          <CardHeader>
            <CardTitle>LLM 对话测试</CardTitle>
            <CardDescription>
              流程：发送提示词 → 服务器按最新保存的 [llm] 配置直调 LLM →
              返回回复（改配置后无需重启即可验证）
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            <div className="space-y-1.5">
              <Label htmlFor="bench-llm-prompt">测试提示词</Label>
              <Textarea
                id="bench-llm-prompt"
                rows={3}
                value={llm.prompt}
                onChange={(event) =>
                  setLlm((previous) => ({ ...previous, prompt: event.target.value }))
                }
              />
            </div>
            <div className="flex items-center gap-3">
              <Button onClick={handleTestLlm} disabled={llm.busy}>
                {llm.busy ? "测试中…" : "发送测试"}
              </Button>
              {llm.busy ? (
                <Badge variant="secondary">测试中</Badge>
              ) : llm.reply !== "" && llm.ok ? (
                <Badge variant="default">正常 · {llm.elapsedMs}ms</Badge>
              ) : llm.reply !== "" ? (
                <Badge variant="destructive">失败</Badge>
              ) : (
                <Badge variant="outline">空闲</Badge>
              )}
            </div>
            <Separator />
            {llm.reply === "" ? (
              <BenchHint text="回复将显示在此（LLM 配置在「配置管理」页 LLM 段修改）" />
            ) : (
              <div className="space-y-2">
                <p
                  className={
                    llm.ok ? "whitespace-pre-wrap" : "whitespace-pre-wrap text-destructive"
                  }
                >
                  {llm.reply}
                </p>
                {llm.ok && llm.elapsedMs > 0 ? (
                  <BenchHint text={`服务端耗时 ${llm.elapsedMs} ms`} />
                ) : null}
                <div className="flex items-center gap-3">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      handleCopy(llm.reply, (hint) =>
                        setLlm((previous) => ({ ...previous, copyHint: hint })),
                      )
                    }
                  >
                    复制
                  </Button>
                  {llm.copyHint !== "" ? (
                    <span className="text-xs text-muted-foreground">{llm.copyHint}</span>
                  ) : null}
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      ) : null}

      {tab === "nlu" ? (
        <Card>
          <CardHeader>
            <CardTitle>NLU 意图测试</CardTitle>
            <CardDescription>
              流程：发送文本 → 服务器按最新保存的 [nlu] 配置构建 TinyBERT 直推 →
              返回意图与耗时（改配置后无需重启即可验证；未配置时回错误原因）
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            <div className="space-y-1.5">
              <Label htmlFor="bench-nlu-input">测试文本</Label>
              <Input
                id="bench-nlu-input"
                value={nlu.input}
                onChange={(event) =>
                  setNlu((previous) => ({ ...previous, input: event.target.value }))
                }
                placeholder="把客厅的灯打开"
              />
            </div>
            <div className="flex items-center gap-3">
              <Button onClick={handleTestNlu} disabled={nlu.busy}>
                {nlu.busy ? "测试中…" : "发送测试"}
              </Button>
              {nlu.busy ? (
                <Badge variant="secondary">测试中</Badge>
              ) : nlu.ok && nlu.intent !== "" ? (
                <Badge variant="default">命中 · {nlu.intent}</Badge>
              ) : nlu.text !== "" ? (
                <Badge variant={nlu.ok ? "outline" : "destructive"}>
                  {nlu.ok ? "未命中" : "失败"}
                </Badge>
              ) : (
                <Badge variant="outline">空闲</Badge>
              )}
            </div>
            <Separator />
            {nluResultText() === "" ? (
              <BenchHint text="结果将显示在此（NLU 配置在「配置管理」页 NLU 段修改）" />
            ) : (
              <div className="space-y-2">
                <p
                  className={
                    nlu.ok ? "whitespace-pre-wrap" : "whitespace-pre-wrap text-destructive"
                  }
                >
                  {nluResultText()}
                </p>
                {nlu.ok && (nlu.tokenizeMs > 0 || nlu.inferMs > 0) ? (
                  <BenchHint text={`分词 ${nlu.tokenizeMs} ms / 推理 ${nlu.inferMs} ms`} />
                ) : null}
                <div className="flex items-center gap-3">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      handleCopy(nluResultText(), (hint) =>
                        setNlu((previous) => ({ ...previous, copyHint: hint })),
                      )
                    }
                  >
                    复制
                  </Button>
                  {nlu.copyHint !== "" ? (
                    <span className="text-xs text-muted-foreground">{nlu.copyHint}</span>
                  ) : null}
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      ) : null}
    </div>
  );
}
