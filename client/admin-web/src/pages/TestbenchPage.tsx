/**
 * 硬件测试台（仅 Tauri 环境挂载，nav `tauriOnly: true`）：五 tab 工作区。
 *
 * 结构对齐旧端 `TestBenchPage.kt:67-81` tab 常量与 `:144-162` 页头：
 * 连接 / ASR 识别 / TTS 合成 / LLM 对话 / NLU 意图。W14a 落地连接 + LLM + NLU
 * 三个纯文本 tab；W14b-1 起 ASR tab 接入本地音频桥真实现；**W14b-2 起 TTS tab
 * 接入下行链路真实现**（server 合成 → 二进制中转 → 本地桥解码播放 → 波形回放）。
 *
 * ASR/TTS 数据流（架构文档 §1.1④，**前端持唯一 `/api/ws` 会话做协议编排**）：
 * - 上行（ASR）：麦克风 → Rust cpal 采集/重采样/Opus 编码 → 本地桥 WS（binary）→
 *   前端 ArrayBuffer 中转 → server `/api/ws`（hello version:1 → unwrap_uplink V1 裸帧）
 *   → test_buf；`asr_test start/stop` 文本帧控制缓冲，`stt` 结果帧回收。
 * - 下行（TTS）：server `/api/ws` 下行 Opus 二进制帧（tts_test 合成结果；下行率由
 *   server hello audio_params 声明——服务端配置 `[audio].downlink_sample_rate`，默认 24k；
 *   v1 = 裸帧）→ 前端中转 → 本地桥（binary）→ Rust 解码播放。`tts start/stop` 文本帧
 *   界定音频流边界；`tts_test` 结果帧回合成耗时/错误；`tts_done` 本地桥事件回整段波形。
 *   **绝对禁止第二条 `/api/ws`**（test_buf 会话级，双 WS = 缓冲分裂）。
 *
 * 与旧端的已登记差异：右栏配置卡不嵌入（TTS 凭据/音色目录探测在「配置管理」页 TTS 段
 * 维护，本页只读快照 + 跳转指引）；「识别耗时」为本端测量的往返；TTS 波形在
 * `tts_done`（本地缓冲排空）时展示，比旧端 tts stop 帧时机晚一次尾帧冲刷（<100ms）。
 * 超时：ASR 30s / LLM 60s / NLU 10s / TTS 60s（`TestBenchState.kt:247/:346/:431/:296`），
 * 文案逐字；server WS 与本地桥均不自动重连。
 */

import { useEffect, useRef, useState, type ReactElement } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { copyText } from "@/lib/clipboard";
import { getConfig } from "@/lib/configApi";
import { getTtsVoices, type Voice } from "@/lib/catalogueApi";
import {
  LANG_OPTIONS,
  GENDER_OPTIONS,
  resolveLang,
  sidOf,
  voiceOptionsOf,
  type VoiceOption,
} from "@/lib/voiceCatalog";
import {
  BridgeSocket,
  TestbenchSocket,
  BRIDGE_UNAVAILABLE_HINT,
  buildTestbenchWsUrl,
  probeTestbenchHandshakeDetail,
  resolveTestbenchToken,
  type BridgeFrame,
  type HelloWire,
  type LlmTestWire,
  type NluTestWire,
  type TtsTestParams,
  type TtsTestWire,
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
const ASR_TIMEOUT_MS = 30_000; // TestBenchState.kt:247（发送识别 30s 未回恢复）
const TTS_TIMEOUT_MS = 60_000; // TestBenchState.kt:296（60s 未收到 tts stop 恢复；服务端合成链预算 30s）

/** ASR 采集阶段机（对齐旧端 TestBenchState asrPhase 五态）。 */
type AsrPhase = "idle" | "starting" | "recording" | "recorded" | "recognizing";

/** ASR 结果块（对齐旧端 TestBenchState asr* 字段组）。 */
interface AsrBenchState {
  phase: AsrPhase;
  /** 录音计秒（wave 事件 200ms 刷新）。 */
  recSeconds: number;
  /** 已录时长（recorded 事件，缓冲长度换算）。 */
  recDuration: number;
  /** 整段波形（64 桶 RMS；recorded 事件填充）。 */
  recWave: number[];
  recProgress: number;
  recPlaying: boolean;
  asrText: string;
  asrElapsedMs: number;
  copyHint: string;
}

const ASR_INITIAL: AsrBenchState = {
  phase: "idle",
  recSeconds: 0,
  recDuration: 0,
  recWave: [],
  recProgress: 0,
  recPlaying: false,
  asrText: "",
  asrElapsedMs: 0,
  copyHint: "",
};

// ====================================================================
// TTS（W14b-2：状态字段对齐旧端 TestBenchState tts* 字段组 + TestBenchAsrTtsPage.kt:139-211）
// ====================================================================

/** TTS 合成模式（由服务端配置 tts.engine 派生；xfyun = 远程，其余按本地呈现）。 */
type TtsMode = "local" | "remote";

/** 讯飞音色分组（按实测目录 gender 分组；custom = 手填 vcn）。 */
type RemoteGroup = "female" | "male" | "custom";

/** 讯飞音色类型（device/voices.rs 的 type 字段；标签对齐 TtsSection TYPE_LABEL 逐字）。 */
const REMOTE_TYPE_LABEL: Record<string, string> = {
  classic: "普通发音人",
  x6: "极速拟人",
};

/** TTS 结果块（对齐旧端 TestBenchState tts* 字段组）。 */
interface TtsBenchState {
  text: string;
  /** 语速输入（字符串承载输入中间态；发送时 parseFloat 失败回 1.0，对齐旧端 toDoubleOrNull）。 */
  speed: string;
  mode: TtsMode;
  lang: "auto" | "zh" | "en";
  /** 本地（Kokoro）：音色性别筛选 + 音色 id + sid（三者联动，对齐旧端 selectVoice）。 */
  voiceGender: "female" | "male" | "en";
  voiceId: string;
  speaker: number;
  /** 远程（讯飞）：服务商固定 xfyun（服务端 vcn 覆盖仅对讯飞有意义，bench.rs:168-209）。 */
  remoteGroup: RemoteGroup;
  remoteType: string;
  remoteVcn: string;
  remoteCustomVcn: string;
  /** 合成中（发送 → tts stop / error / 超时）。 */
  busy: boolean;
  /** 下行音频流进行中（tts start → tts stop；二进制中转的开关）。 */
  speaking: boolean;
  /** 合成完成且本地缓冲排空（tts_done）——波形与回听就绪。 */
  ready: boolean;
  engine: string;
  /** 服务端合成耗时（ok 帧 text="…ms" 解析）。 */
  elapsedMs: number;
  error: string;
  wave: number[];
  duration: number;
  progress: number;
  playing: boolean;
}

const TTS_INITIAL: TtsBenchState = {
  text: "",
  speed: "1.0",
  mode: "local",
  lang: "auto",
  voiceGender: "female",
  voiceId: "zf_001",
  speaker: 3,
  remoteGroup: "female",
  remoteType: "classic",
  remoteVcn: "",
  remoteCustomVcn: "",
  busy: false,
  speaking: false,
  ready: false,
  engine: "",
  elapsedMs: 0,
  error: "",
  wave: [],
  duration: 0,
  progress: 0,
  playing: false,
};

/** 按性别落第一个音色并联动语言（对齐旧端 selectVoice + 性别联动，TestBenchAsrTtsPage.kt:228-232）。 */
function pickVoice(gender: "female" | "male" | "en"): { voiceId: string; speaker: number } {
  const first: VoiceOption | undefined = voiceOptionsOf(gender)[0];
  return { voiceId: first?.id ?? "zf_001", speaker: first?.sid ?? 3 };
}

/** 从配置根/配置段读取子段（JsonObject → 普通对象）。 */
function configSection(
  root: Record<string, unknown> | null,
  key: string,
): Record<string, unknown> | null {
  const value = root?.[key];
  if (typeof value === "object" && value !== null && !Array.isArray(value)) {
    return value as Record<string, unknown>;
  }
  return null;
}

/** 配置段的字符串字段（缺失/非串按 ""）。 */
function configString(section: Record<string, unknown> | null, key: string): string {
  const value = section?.[key];
  return typeof value === "string" ? value : "";
}

/** 配置段的数字字段（缺失/非有限数按 null）。 */
function configNumber(section: Record<string, unknown> | null, key: string): number | null {
  const value = section?.[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/** 秒 → "12.3s"（录音计秒十进制显示，对齐旧端 formatSeconds）。 */
function formatSeconds(seconds: number): string {
  const tenth = Math.max(0, Math.floor(seconds * 10));
  return `${Math.floor(tenth / 10)}.${tenth % 10}s`;
}

/** 秒 → "m:ss"（播放器时长，对齐旧端 formatDuration）。 */
function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** 波形条（对齐旧端 GearWaveform 语义：64 桶 + 播放进度点亮 + 点击试听）。 */
function WaveformBar({
  wave,
  progress,
  playing,
  durationText,
  disabled,
  onToggle,
}: {
  wave: readonly number[];
  progress: number;
  playing: boolean;
  durationText: string;
  disabled: boolean;
  onToggle: () => void;
}): ReactElement {
  const bars = wave.length > 0 ? wave : [];
  return (
    <div className="flex items-center gap-3">
      <Button variant="outline" size="sm" onClick={onToggle} disabled={disabled}>
        {playing ? "停止试听" : "试听"}
      </Button>
      <div className="flex h-10 flex-1 items-end gap-px" aria-hidden="true">
        {bars.map((value, index) => {
          const lit = playing && index / bars.length <= progress;
          return (
            <div
              key={index}
              className={
                lit ? "min-w-[2px] flex-1 bg-primary" : "min-w-[2px] flex-1 bg-muted-foreground/40"
              }
              style={{ height: `${Math.max(6, value * 100)}%` }}
            />
          );
        })}
      </div>
      <span className="text-xs text-muted-foreground">{durationText}</span>
    </div>
  );
}

/** 灰字说明行（对齐旧端 HintLine 零件：结果块的占位/耗时明细）。 */
function BenchHint({ text }: { text: string }): ReactElement {
  return <p className="text-xs text-muted-foreground">{text}</p>;
}

/** 带标签的下拉（对齐旧端 labeledSelect；Radix Select 不允许空串 item value）。 */
function BenchSelect({
  id,
  label,
  value,
  onChange,
  options,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: readonly { value: string; label: string }[];
}): ReactElement {
  // 当前值不在选项内时追加兜底项（如实显示服务端/配置值，不自动纠正）。
  const known = options.some((option) => option.value === value);
  const items = known ? options : [{ value, label: `${value}（未知值，如实呈现）` }, ...options];
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger id={id} className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {items.map((option) => (
            <SelectItem key={option.value} value={option.value}>
              {option.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
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
  const [asr, setAsr] = useState<AsrBenchState>(ASR_INITIAL);
  const [tts, setTts] = useState<TtsBenchState>(TTS_INITIAL);
  // 服务端配置快照（TTS tab 打开时拉一次；模式/默认值派生用，不改配置）。
  const [voices, setVoices] = useState<readonly Voice[] | null>(null);
  const [voiceCatalogueHint, setVoiceCatalogueHint] = useState("");

  const socketRef = useRef<TestbenchSocket | null>(null);
  const llmTimerRef = useRef<number | null>(null);
  const nluTimerRef = useRef<number | null>(null);
  // 本地音频桥（按需连接：首次「开始录音」或 TTS 下行时建立，保持复用；断开不自动重连）。
  const bridgeRef = useRef<BridgeSocket | null>(null);
  const asrTimerRef = useRef<number | null>(null);
  const ttsTimerRef = useRef<number | null>(null);
  // 「发送识别」起点时间戳：识别耗时 = 发送 asr_test stop → 收到 stt 的本端往返。
  const asrSentAtRef = useRef<number | null>(null);
  // 下行采样率（server hello audio_params；默认 24k——服务端 [audio].downlink_sample_rate 默认值）。
  const downlinkSrRef = useRef<number>(24_000);
  // 下行音频流开关（tts start → tts stop；二进制中转的闸门）。
  const speakingRef = useRef<boolean>(false);
  // 服务端回报的引擎名（tts_test ok 帧；tts_done 时拼「可回听」文案用）。
  const ttsEngineRef = useRef<string>("");
  // 试听/回放来源（play_state 路由：duration>0 的帧按此分发到 ASR 或 TTS 状态）。
  const playbackSourceRef = useRef<"rec" | "tts" | null>(null);

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
      bridgeRef.current?.close();
      bridgeRef.current = null;
      for (const timer of [llmTimerRef, nluTimerRef, asrTimerRef, ttsTimerRef]) {
        if (timer.current !== null) {
          window.clearTimeout(timer.current);
        }
      }
    };
  }, []);

  // TTS tab 打开时拉一次服务端配置快照（只读；模式与本地默认值派生，不改配置）。
  // 音色目录一并拉取（GET /api/tts/voices 只读；探测动作本身在「配置管理」页 TTS 段）。
  useEffect(() => {
    if (tab !== "tts" || voices !== null || voiceCatalogueHint !== "") {
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const config = await getConfig();
        if (cancelled) {
          return;
        }
        const ttsSection = configSection(config, "tts");
        const engine = configString(ttsSection, "engine");
        const kokoro = configSection(ttsSection, "kokoro");
        const xfyun = configSection(ttsSection, "xfyun");
        const speaker = configNumber(ttsSection, "speaker");
        const lang = configString(kokoro, "lang");
        const vcn = configString(xfyun, "voice");
        setTts((previous) => {
          // 配置 sid ↔ 音色目录联动（voiceId/gender 从 sid 反推；未知 sid 保留默认）。
          let voiceId = previous.voiceId;
          let voiceGender = previous.voiceGender;
          if (speaker !== null) {
            for (const gender of ["female", "male", "en"] as const) {
              const match = voiceOptionsOf(gender).find((option) => option.sid === speaker);
              if (match) {
                voiceId = match.id;
                voiceGender = gender;
                break;
              }
            }
          }
          const langValue = lang === "zh" || lang === "en" ? lang : "auto";
          return {
            ...previous,
            mode: engine === "xfyun" ? "remote" : "local",
            voiceId,
            voiceGender,
            speaker: speaker ?? previous.speaker,
            lang: langValue,
            remoteVcn: vcn !== "" ? vcn : previous.remoteVcn,
          };
        });
      } catch {
        // 配置读取失败：按本地默认呈现，不阻塞测试台（模式可在发送时被服务端纠正）。
      }
      try {
        const result = await getTtsVoices();
        if (cancelled) {
          return;
        }
        setVoices(result.voices);
      } catch {
        if (cancelled) {
          return;
        }
        setVoices([]);
        setVoiceCatalogueHint(
          "音色目录不可用（服务端未启用音色目录能力或尚未探测）；TTS 凭据与音色目录探测在「配置管理」页 TTS 段维护。",
        );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [tab, voices, voiceCatalogueHint]);

  /** 三态连接按钮：未连=连接、连接中=菊花拦截、已连=断开。 */
  const handleConnectToggle = (): void => {
    if (connectionState === "connecting") {
      return;
    }
    if (connectionState === "connected") {
      // 手动断开：录音中先停本地桥采集、TTS 下行中先停播放（二进制无处可转）。
      if (asr.phase === "recording") {
        bridgeRef.current?.stopCapture();
        setAsr((previous) => ({ ...previous, phase: "idle", recSeconds: 0 }));
      }
      if (speakingRef.current) {
        speakingRef.current = false;
        bridgeRef.current?.stopPlayback();
        setTts((previous) => ({ ...previous, speaking: false, busy: false }));
      }
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
        // server WS 断开时若在录音：本地桥立即停采集（二进制无处可转，避免悬挂录音）。
        setAsr((previous) => {
          if (previous.phase === "recording") {
            bridgeRef.current?.stopCapture();
            return { ...previous, phase: "idle", recSeconds: 0 };
          }
          return previous;
        });
        // TTS 下行中：停桥播放会话（音频流已断，残留会话只会静音）。
        if (speakingRef.current) {
          speakingRef.current = false;
          bridgeRef.current?.stopPlayback();
          setTts((previous) => ({ ...previous, speaking: false, busy: false }));
        }
        if (ttsTimerRef.current !== null) {
          window.clearTimeout(ttsTimerRef.current);
          ttsTimerRef.current = null;
        }
      },
      onHello: (hello: HelloWire) => {
        // server hello 携带下行音频参数（gateway/ws/mod.rs:435-446）：
        // 下行采样率由服务端配置决定，解码与 tts_begin 以此为准（缺省按默认 24k）。
        if (hello.audio !== null) {
          downlinkSrRef.current = hello.audio.sampleRate;
        }
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
      onSttResult: (result) => {
        if (asrTimerRef.current !== null) {
          window.clearTimeout(asrTimerRef.current);
          asrTimerRef.current = null;
        }
        // 识别耗时 = 发送 asr_test stop → 收到 stt 的本端往返（server Stt 帧不带耗时）。
        const elapsed = asrSentAtRef.current === null ? 0 : Date.now() - asrSentAtRef.current;
        asrSentAtRef.current = null;
        setAsr((previous) => ({
          ...previous,
          phase: previous.phase === "recognizing" ? "recorded" : "idle",
          asrText: result.text,
          asrElapsedMs: elapsed,
        }));
        setStatusMsg(elapsed > 0 ? `识别完成（耗时 ${elapsed}ms）` : "识别完成");
      },
      onTtsState: ({ state }: { state: "start" | "stop" }) => {
        if (state === "start") {
          // 下行音频流开始：置 speaking 闸门（二进制中转开）→ 桥就绪后发 tts_begin
          //（下行率取 server hello audio_params；Rust 建解码器 + 输出流等数据）。
          speakingRef.current = true;
          setTts((previous) => ({
            ...previous,
            speaking: true,
            ready: false,
            progress: 0,
            playing: false,
            wave: [],
            duration: 0,
          }));
          ensureBridge((bridge) => {
            bridge.ttsBegin(downlinkSrRef.current);
          });
          return;
        }
        // tts stop：合成收尾（ok 帧已先到，引擎名/耗时已记）；通知 Rust 缓冲排空后回
        // tts_done（波形与「播放完成」文案在那时展示，比旧端 stop 帧晚一次尾帧冲刷）。
        speakingRef.current = false;
        setTts((previous) => ({ ...previous, speaking: false, busy: false }));
        bridgeRef.current?.ttsEnd();
      },
      onTtsTestResult: (result: TtsTestWire) => {
        const engine = result.engine ?? "";
        ttsEngineRef.current = engine;
        if (result.state === "error") {
          // 服务端合成失败（引擎切换失败/凭据错误/网络等，含 30s 预算超时原文）：
          // 错误原文如实呈现（对齐旧端 ttsError / statusMsg）。
          if (ttsTimerRef.current !== null) {
            window.clearTimeout(ttsTimerRef.current);
            ttsTimerRef.current = null;
          }
          speakingRef.current = false;
          bridgeRef.current?.stopPlayback(); // 清掉可能残留的下行会话
          const error = result.text ?? "合成失败";
          setTts((previous) => ({
            ...previous,
            busy: false,
            speaking: false,
            engine,
            error,
          }));
          setStatusMsg(`TTS 失败：${error}`);
          return;
        }
        // ok 帧：只记引擎名与耗时（音频还在流式下发；按钮在 tts stop 解锁，对齐旧端）。
        const elapsedMs = Number.parseInt(result.text ?? "", 10);
        setTts((previous) => ({
          ...previous,
          engine,
          elapsedMs: Number.isFinite(elapsedMs) ? elapsedMs : 0,
          error: "",
        }));
      },
      onBinary: (data: ArrayBuffer) => {
        // 下行 Opus 中转：tts start → tts stop 之间的二进制帧 → 本地桥解码播放。
        // （唯一 server WS 会话；本地桥只走音频数据，双 /api/ws 绝对禁止。）
        if (speakingRef.current) {
          bridgeRef.current?.sendBinary(data);
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

  // ====================================================================
  // ASR（W14b-1：本地音频桥 + asr_test 编排；状态机对齐 TestBenchState.kt:182-253）
  // ====================================================================

  /** 本地桥事件帧 → ASR/TTS 状态机推进。 */
  const handleBridgeFrame = (frame: BridgeFrame): void => {
    switch (frame.type) {
      case "started": {
        // 录音真正开始（原生确认，非猜测延时）：发 asr_test start 进入缓冲，计秒开始。
        const sent = socketRef.current?.sendAsrTest("start") ?? false;
        if (!sent) {
          bridgeRef.current?.stopCapture();
          setConnectionState("error");
          setAsr((previous) => ({ ...previous, phase: "idle" }));
          setStatusMsg("请先在「连接」卡片建立连接");
          return;
        }
        setAsr((previous) => ({ ...previous, phase: "recording", recSeconds: 0 }));
        setStatusMsg("录音中…说完点「停止录音」");
        break;
      }
      case "wave":
        // 录音中计秒（200ms 推送）；波形只在停止后整段展示（对齐旧端）。
        setAsr((previous) => ({ ...previous, recSeconds: frame.seconds }));
        break;
      case "recorded":
        setAsr((previous) => ({
          ...previous,
          phase: "recorded",
          recDuration: frame.seconds,
          recWave: frame.buckets,
          recProgress: 0,
          recPlaying: false,
        }));
        setStatusMsg(`已停止（录了 ${formatSeconds(frame.seconds)}），可试听后点「发送识别」`);
        break;
      case "play_state":
        // duration=0 是 TTS 下行直播会话的节流帧（流式时长未知，UI 不消费）；
        // duration>0 是试听/回放进度，按发起来源分发（单播放器语义，对齐旧端）。
        if (frame.duration <= 0) {
          break;
        }
        if (playbackSourceRef.current === "tts") {
          setTts((previous) => ({
            ...previous,
            playing: frame.playing,
            progress: frame.playing && frame.duration > 0 ? frame.position / frame.duration : 0,
          }));
        } else {
          setAsr((previous) => ({
            ...previous,
            recPlaying: frame.playing,
            recProgress: frame.playing && frame.duration > 0 ? frame.position / frame.duration : 0,
          }));
        }
        break;
      case "tts_done":
        // TTS 下行缓冲排空：整段波形 + 时长就绪（对齐旧端 tts stop 回调的
        // 「播放完成，可回听」+ refreshWave，晚一次尾帧冲刷，已登记差异）。
        playbackSourceRef.current = "tts";
        setTts((previous) => ({
          ...previous,
          ready: true,
          wave: frame.buckets,
          duration: frame.seconds,
          progress: 0,
          playing: false,
        }));
        setStatusMsg(`TTS 播放完成，可回听（${ttsEngineRef.current || "未知引擎"}）`);
        break;
      case "error":
        // 权限拒绝/设备不可用：phase 回 idle，Rust 侧锚点文案逐字呈现。
        setAsr((previous) =>
          previous.phase === "recognizing" ? previous : { ...previous, phase: "idle" },
        );
        setStatusMsg(`录音启动失败：${frame.message}`);
        break;
    }
  };

  /**
   * 按需建立本地桥连接（ASR 录音与 TTS 下行共用一条桥连接）；就绪后回调 onReady。
   * 失败（桥未注入 / 连接失败）如实提示并回退调用方的状态。
   */
  const ensureBridge = (onReady: (bridge: BridgeSocket) => void): void => {
    if (bridgeRef.current?.isOpen) {
      onReady(bridgeRef.current);
      return;
    }
    const bridge = new BridgeSocket({
      onOpen: () => {
        if (bridgeRef.current !== null) {
          onReady(bridgeRef.current);
        }
      },
      onClose: (code) => {
        bridgeRef.current = null;
        setAsr((previous) =>
          previous.phase === "starting" || previous.phase === "recording"
            ? { ...previous, phase: "idle", recSeconds: 0 }
            : previous,
        );
        if (speakingRef.current) {
          speakingRef.current = false;
          setTts((previous) => ({ ...previous, speaking: false, busy: false }));
        }
        setStatusMsg(`连接失败: 本地音频桥已断开（close code ${code}）`);
      },
      onFrame: (frame) => handleBridgeFrame(frame),
      onBinary: (data) => {
        // 上行 Opus 帧中转：唯一 server WS 会话（双 /api/ws = test_buf 缓冲分裂，绝对禁止）。
        socketRef.current?.sendBinary(data);
      },
    });
    const opened = bridge.open();
    if (!opened) {
      // 桥未注入：按调用方场景回退（ASR 空闲态 / TTS 取消下行闸门）。
      setAsr((previous) => ({ ...previous, phase: "idle" }));
      speakingRef.current = false;
      setTts((previous) => ({ ...previous, speaking: false }));
      setStatusMsg(`录音启动失败：${BRIDGE_UNAVAILABLE_HINT}`);
      return;
    }
    bridgeRef.current = bridge;
  };

  /** ASR 主按钮：承载 开始/停止/重新录音 三种动作（starting/recognizing 禁用）。 */
  const handleAsrButton = (): void => {
    if (asr.phase === "starting" || asr.phase === "recording" || asr.phase === "recognizing") {
      if (asr.phase === "recording") {
        bridgeRef.current?.stopCapture(); // recorded 事件推进状态机
      }
      return;
    }
    if (connectionState !== "connected") {
      setStatusMsg("请先在「连接」卡片建立连接");
      return;
    }
    // idle / recorded → 重新开始（recorded 时 Rust 侧 start 会清旧缓冲，对齐旧端 startAsr 复位）。
    setAsr((previous) => ({
      ...ASR_INITIAL,
      phase: "starting",
      recWave: previous.phase === "recorded" ? [] : previous.recWave,
    }));
    setStatusMsg("正在启动录音…");
    if (bridgeRef.current?.isOpen) {
      bridgeRef.current.startCapture();
    } else {
      ensureBridge((bridge) => {
        bridge.startCapture();
      });
    }
  };

  /** 发送识别（仅已录时出现）：asr_test stop → 30s 超时兜底 → stt 回包（见 onSttResult）。 */
  const handleSendAsr = (): void => {
    if (asr.phase !== "recorded") {
      return;
    }
    setAsr((previous) => ({ ...previous, phase: "recognizing" }));
    setStatusMsg("识别中…");
    asrSentAtRef.current = Date.now();
    const sent = socketRef.current?.sendAsrTest("stop") ?? false;
    if (!sent) {
      asrSentAtRef.current = null;
      setAsr((previous) => ({ ...previous, phase: "recorded" }));
      setConnectionState("error");
      setStatusMsg("请先在「连接」卡片建立连接");
      return;
    }
    asrTimerRef.current = window.setTimeout(() => {
      asrTimerRef.current = null;
      asrSentAtRef.current = null;
      setAsr((previous) => ({ ...previous, phase: "recorded" }));
      setStatusMsg("识别超时：请检查服务器日志或重试");
    }, ASR_TIMEOUT_MS);
  };

  /** 试听/停止播放（单播放器语义；进度由 play_state 推送驱动）。 */
  const handleTogglePlayback = (): void => {
    if (asr.phase === "recording" || asr.phase === "starting") {
      return;
    }
    if (asr.recPlaying) {
      bridgeRef.current?.stopPlayback();
    } else {
      playbackSourceRef.current = "rec";
      bridgeRef.current?.playRecorded();
    }
  };

  // ====================================================================
  // TTS（W14b-2：tts_test 编排 + 下行中转 + 回放；对齐 TestBenchState.kt:259-304）
  // ====================================================================

  /** 远程模式实际下发的 vcn（custom 组 = 手填；目录组 = 选中发音人）。 */
  const effectiveVcn = (): string => {
    if (tts.mode !== "remote") {
      return "";
    }
    return tts.remoteGroup === "custom" ? tts.remoteCustomVcn : tts.remoteVcn;
  };

  /** 发起合成：前置校验（文案逐字）→ tts_test → 60s 超时兜底。 */
  const handleTestTts = (): void => {
    if (tts.busy || tts.speaking) {
      return;
    }
    if (connectionState !== "connected") {
      setStatusMsg("请先在「连接」卡片建立连接");
      return;
    }
    if (tts.text.trim() === "") {
      setStatusMsg("请输入要合成的文字");
      return;
    }
    setTts((previous) => ({
      ...previous,
      busy: true,
      ready: false,
      error: "",
      progress: 0,
      playing: false,
      wave: [],
      duration: 0,
    }));
    setStatusMsg("合成中…");
    // 语速解析失败回 1.0（对齐旧端 toDoubleOrNull ?: 1.0，TestBenchState.kt:272）。
    const speed = Number.parseFloat(tts.speed);
    const baseParams: TtsTestParams = {
      text: tts.text,
      speaker: tts.speaker,
      lang: resolveLang(tts.lang),
      speed: Number.isFinite(speed) && speed > 0 ? speed : 1.0,
    };
    // 远程音色随请求下发（无需先保存配置，对齐旧端 speak 调用点语义）。
    const vcn = effectiveVcn().trim();
    const params: TtsTestParams =
      tts.mode === "remote" && vcn !== "" ? { ...baseParams, vcn } : baseParams;
    const sent = socketRef.current?.sendTtsTest(params) ?? false;
    if (!sent) {
      setTts((previous) => ({ ...previous, busy: false }));
      setConnectionState("error");
      setStatusMsg("请先在「连接」卡片建立连接");
      return;
    }
    ttsTimerRef.current = window.setTimeout(() => {
      ttsTimerRef.current = null;
      speakingRef.current = false;
      bridgeRef.current?.stopPlayback();
      setTts((previous) => ({
        ...previous,
        busy: false,
        speaking: false,
        error: "未收到服务器结果，请查看服务器日志",
      }));
      setStatusMsg("合成超时：未收到服务器结果，请查看服务器日志（可重试）");
    }, TTS_TIMEOUT_MS);
  };

  /** TTS 回放/停止（tts_done 后可用；进度由 play_state 推送驱动）。 */
  const handleTtsTogglePlayback = (): void => {
    if (!tts.ready || tts.busy) {
      return;
    }
    if (tts.playing) {
      bridgeRef.current?.stopPlayback();
    } else {
      playbackSourceRef.current = "tts";
      bridgeRef.current?.playTts();
    }
  };

  /** 本地音色联动（对齐旧端 selectVoice + 性别联动语言）。 */
  const handleSelectGender = (gender: "female" | "male" | "en"): void => {
    const picked = pickVoice(gender);
    setTts((previous) => ({
      ...previous,
      voiceGender: gender,
      voiceId: picked.voiceId,
      speaker: picked.speaker,
      lang: gender === "en" ? "en" : "zh",
    }));
  };

  /** 本地音色选择（sid 同步）。 */
  const handleSelectVoice = (id: string): void => {
    const sid = sidOf(id);
    setTts((previous) => ({
      ...previous,
      voiceId: id,
      speaker: sid ?? previous.speaker,
    }));
  };

  /** 远程音色分组切换（custom = 手填；否则按 gender×type 过滤目录）。 */
  const remoteGroupVoices = (): readonly Voice[] => {
    if (voices === null || tts.remoteGroup === "custom") {
      return [];
    }
    return voices.filter(
      (voice) => voice.gender === tts.remoteGroup && voice.type === tts.remoteType,
    );
  };

  const wsUrl = buildTestbenchWsUrl(null);

  return (
    <div className="mx-auto w-full max-w-4xl space-y-6">
      {/* 页头（对齐旧端 BenchHeader：标题 + statusMsg 状态行，空时给固定说明） */}
      <div className="space-y-1">
        <h2 className="text-2xl font-semibold tracking-tight">测试台</h2>
        <p className="text-sm text-muted-foreground">
          {statusMsg === ""
            ? "连接服务端后测试录音识别 / TTS 合成 / LLM 对话 / NLU 意图。"
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
            <CardTitle>ASR 识别测试</CardTitle>
            <CardDescription>
              流程：录音 → 停止后可反复试听 → 发送识别（SenseVoice 自动检测语言）
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            {/* 主按钮：五态文案随 phase 切换（对齐旧端 TestBenchAsrTtsPage.kt:63-68）；发送识别仅在已录时出现 */}
            <div className="flex items-center gap-3">
              <Button
                onClick={handleAsrButton}
                disabled={asr.phase === "starting" || asr.phase === "recognizing"}
                variant={asr.phase === "recording" ? "destructive" : "default"}
              >
                {asr.phase === "starting"
                  ? "正在启动…"
                  : asr.phase === "recording"
                    ? "停止录音"
                    : asr.phase === "recognizing"
                      ? "识别中…"
                      : asr.phase === "recorded"
                        ? "重新录音"
                        : "开始录音"}
              </Button>
              {asr.phase === "recorded" ? <Button onClick={handleSendAsr}>发送识别</Button> : null}
            </div>
            {/* 状态徽标：独立一行（对齐旧端 :99-105） */}
            <div className="flex items-center gap-2">
              {asr.phase === "recording" ? (
                <Badge variant="secondary">录音中 {formatSeconds(asr.recSeconds)}</Badge>
              ) : asr.phase === "recognizing" ? (
                <Badge variant="secondary">识别中</Badge>
              ) : asr.phase === "recorded" ? (
                <Badge variant="default">
                  已录 {formatDuration(asr.recDuration)}，可试听/发送识别
                </Badge>
              ) : (
                <Badge variant="outline">空闲</Badge>
              )}
            </div>
            {/* 试听播放器（已录且整段波形就绪时显示；录音中不显示，对齐旧端 :108-117） */}
            {asr.phase === "recorded" && asr.recWave.length > 0 ? (
              <WaveformBar
                wave={asr.recWave}
                progress={asr.recProgress}
                playing={asr.recPlaying}
                durationText={formatDuration(asr.recDuration)}
                disabled={false}
                onToggle={handleTogglePlayback}
              />
            ) : null}
            <Separator />
            {asr.asrText === "" ? (
              <BenchHint text="识别结果将显示在此" />
            ) : (
              <div className="space-y-2">
                <p className="whitespace-pre-wrap">{asr.asrText}</p>
                {asr.asrElapsedMs > 0 ? (
                  <BenchHint text={`识别耗时 ${asr.asrElapsedMs} ms（发送→收到结果）`} />
                ) : null}
                <div className="flex items-center gap-3">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      handleCopy(asr.asrText, (hint) =>
                        setAsr((previous) => ({ ...previous, copyHint: hint })),
                      )
                    }
                  >
                    复制
                  </Button>
                  {asr.copyHint !== "" ? (
                    <span className="text-xs text-muted-foreground">{asr.copyHint}</span>
                  ) : null}
                </div>
              </div>
            )}
            <div className="flex flex-wrap gap-2 border-t border-border pt-3">
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
            <CardTitle>TTS 合成测试</CardTitle>
            <CardDescription>
              流程：输入文字 → 服务器按最新保存的 TTS 配置合成（改配置后无需重启）→ 下行音频实时播放
              → 播完可回听；凭据与音色目录探测在「配置管理」页 TTS 段维护
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            <div className="space-y-1.5">
              <Label htmlFor="bench-tts-text">合成文字</Label>
              <Textarea
                id="bench-tts-text"
                rows={3}
                value={tts.text}
                onChange={(event) =>
                  setTts((previous) => ({ ...previous, text: event.target.value }))
                }
                placeholder="输入要合成的文字"
              />
            </div>
            {tts.mode === "local" ? (
              <div className="grid gap-3 sm:grid-cols-3">
                <BenchSelect
                  id="bench-tts-lang"
                  label="语言"
                  value={tts.lang}
                  onChange={(value) => {
                    if (value === "auto" || value === "zh" || value === "en") {
                      setTts((previous) => ({ ...previous, lang: value }));
                    }
                  }}
                  options={LANG_OPTIONS.map((option) => ({ ...option }))}
                />
                <BenchSelect
                  id="bench-tts-gender"
                  label="音色性别"
                  value={tts.voiceGender}
                  onChange={(value) => {
                    if (value === "female" || value === "male" || value === "en") {
                      handleSelectGender(value);
                    }
                  }}
                  options={GENDER_OPTIONS.map((option) => ({ ...option }))}
                />
                <BenchSelect
                  id="bench-tts-voice"
                  label="音色"
                  value={tts.voiceId}
                  onChange={handleSelectVoice}
                  options={voiceOptionsOf(tts.voiceGender).map((option) => ({
                    value: option.id,
                    label: `${option.id}（sid ${option.sid}）`,
                  }))}
                />
              </div>
            ) : (
              <div className="space-y-3">
                <div className="grid gap-3 sm:grid-cols-2">
                  <BenchSelect
                    id="bench-tts-engine"
                    label="服务商"
                    value="xfyun"
                    onChange={() => undefined}
                    options={[{ value: "xfyun", label: "讯飞（xfyun）" }]}
                  />
                  <BenchSelect
                    id="bench-tts-group"
                    label="音色分组"
                    value={tts.remoteGroup}
                    onChange={(value) => {
                      if (value === "female" || value === "male" || value === "custom") {
                        // 切组回落该组第一个可用音色（无目录时清空 vcn，避免带旧值合成）。
                        const group = value;
                        const first = (voices ?? []).find(
                          (voice) => voice.gender === group && voice.type === tts.remoteType,
                        );
                        setTts((previous) => ({
                          ...previous,
                          remoteGroup: group,
                          remoteType:
                            group === "custom"
                              ? previous.remoteType
                              : (voices ?? []).some(
                                    (voice) =>
                                      voice.gender === group && voice.type === previous.remoteType,
                                  )
                                ? previous.remoteType
                                : ((voices ?? []).find((voice) => voice.gender === group)?.type ??
                                  previous.remoteType),
                          remoteVcn: first?.vcn ?? "",
                        }));
                      }
                    }}
                    options={[
                      { value: "female", label: "女声" },
                      { value: "male", label: "男声" },
                      { value: "custom", label: "自定义" },
                    ]}
                  />
                </div>
                {tts.remoteGroup !== "custom" ? (
                  <div className="grid gap-3 sm:grid-cols-2">
                    <BenchSelect
                      id="bench-tts-type"
                      label="音色类型"
                      value={tts.remoteType}
                      onChange={(value) => {
                        const first = (voices ?? []).find(
                          (voice) => voice.gender === tts.remoteGroup && voice.type === value,
                        );
                        setTts((previous) => ({
                          ...previous,
                          remoteType: value,
                          remoteVcn: first?.vcn ?? "",
                        }));
                      }}
                      options={[...new Set((voices ?? []).map((voice) => voice.type))].map(
                        (type) => ({
                          value: type,
                          label: REMOTE_TYPE_LABEL[type] ?? type,
                        }),
                      )}
                    />
                    <BenchSelect
                      id="bench-tts-vcn"
                      label="发音人（vcn）"
                      value={tts.remoteVcn}
                      onChange={(value) =>
                        setTts((previous) => ({ ...previous, remoteVcn: value }))
                      }
                      options={remoteGroupVoices().map((voice) => ({
                        value: voice.vcn,
                        label: `${voice.vcn}（${voice.name}）`,
                      }))}
                    />
                  </div>
                ) : (
                  <div className="space-y-1.5">
                    <Label htmlFor="bench-tts-custom-vcn">voice（手填 vcn）</Label>
                    <Input
                      id="bench-tts-custom-vcn"
                      value={tts.remoteCustomVcn}
                      onChange={(event) =>
                        setTts((previous) => ({ ...previous, remoteCustomVcn: event.target.value }))
                      }
                      placeholder="手填讯飞发音人 vcn"
                    />
                  </div>
                )}
                {voiceCatalogueHint !== "" ? (
                  <BenchHint text={voiceCatalogueHint} />
                ) : (
                  <BenchHint text="TTS 密钥与音色目录探测在「配置管理」页 TTS 段维护；测试台合成会直接使用上方选中的音色（无需先保存配置）。" />
                )}
              </div>
            )}
            <div className="space-y-1.5">
              <Label htmlFor="bench-tts-speed">语速</Label>
              <Input
                id="bench-tts-speed"
                className="w-32"
                value={tts.speed}
                onChange={(event) =>
                  setTts((previous) => ({ ...previous, speed: event.target.value }))
                }
                placeholder="1.0"
              />
            </div>
            <div className="flex items-center gap-3">
              <Button
                onClick={handleTestTts}
                disabled={
                  tts.busy ||
                  tts.speaking ||
                  asr.phase === "recording" ||
                  asr.phase === "starting" ||
                  asr.phase === "recognizing"
                }
              >
                {tts.busy || tts.speaking ? "合成中…" : "合成并播放"}
              </Button>
              {tts.engine !== "" ? (
                <Badge variant={tts.error !== "" ? "destructive" : "default"}>
                  {tts.error !== "" ? `${tts.engine} · 失败` : `${tts.engine} · 就绪`}
                </Badge>
              ) : null}
              {tts.busy || tts.speaking ? (
                <Badge variant="secondary">合成中</Badge>
              ) : tts.ready ? (
                <Badge variant="outline">
                  已合成 {formatDuration(tts.duration)}
                  {tts.elapsedMs > 0 ? `（服务端 ${tts.elapsedMs}ms）` : ""}
                </Badge>
              ) : null}
            </div>
            {tts.error !== "" ? (
              <p className="whitespace-pre-wrap text-sm text-destructive">合成失败：{tts.error}</p>
            ) : null}
            {tts.ready && tts.wave.length > 0 ? (
              <WaveformBar
                wave={tts.wave}
                progress={tts.progress}
                playing={tts.playing}
                durationText={formatDuration(tts.duration)}
                disabled={tts.busy}
                onToggle={handleTtsTogglePlayback}
              />
            ) : null}
            <div className="flex flex-wrap gap-2 border-t border-border pt-3">
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
