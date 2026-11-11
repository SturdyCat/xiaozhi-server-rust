/**
 * 测试台 WS 客户端：连接 `/api/ws`（设备协议通道），承载 LLM / NLU 纯文本测试。
 *
 * ## 端点与鉴权
 * - `/api/ws` 是设备 WebSocket 协议端点（`gateway/ws/mod.rs`），**不是**管理台实时
 *   通道 `/api/admin/ws`（`api.ts` 的 getWebSocketUrl 固定指向后者，不能复用）。
 *   地址从统一基址（`getApiBaseUrl()`）推导：http→ws / https→wss，语义对齐旧端
 *   `unifiedWsEndpoint`（WS 端点始终跟随统一服务器基址）。
 * - 浏览器 WebSocket 不能自定义请求头 → token 走 `?token=` 查询参数，服务端
 *   **明确允许**（`ws/mod.rs:391-395`：「浏览器 WebSocket 无法自定义请求头，
 *   允许 ?token= 查询参数兜底」；设备侧仍走 Authorization 头）。token 为空时不拼。
 *
 * ## hello 帧（逐字段对齐 `XiaoZhiModule.m:104-117`）
 * `{type:"hello", version:1, test:true, audio_params:{format:"opus",sample_rate:16000,
 * channels:1,frame_duration:60}}`。`test:true` 标记本连接为管理端测试台——服务端
 * **仅受理 test 会话**的 `llm_test`/`nlu_test`（`bench.rs:422/:489` 非测试会话静默忽略）。
 * `audio_params` 虽为可省字段（`ClientHello` 全 `#[serde(default)]`），仍按原生模块
 * 原样发送：服务端 hello 回包的 audio_params 被用作下行解码参数，保持完整声明
 * 最贴近旧端行为、零成本（纯文本测试不下发音频，只是声明）。
 *
 * ## 测试帧 wire（权威 `workflow/protocol/protocol.rs` + `session/bench.rs`）
 * - 上行：`{type:"llm_test", text}` / `{type:"nlu_test", text}`（`session_id` 可省，
 *   `#[serde(default)]`；ClientMessage `tag="type"` + `rename_all="snake_case"`）。
 * - 下行结果帧与上行同 `type` 标签：
 *   `llm_test: {session_id, state:"ok"|"error", text?, elapsed_ms?}`（text=回复正文或
 *   可读错误；elapsed_ms=服务端直调 LLM 总耗时）；`state=ok` 但 text 为空是合法回包
 *   （如模型只回工具调用），由 UI 层按「（LLM 返回了空回复）」兜底。
 *   `nlu_test: {session_id, state, intent?, confidence?, tokenize_ms?, infer_ms?, text?}`
 *   （低于 `bert_threshold` 时 intent=None 并在 text 说明；error 帧 text=可读原因）。
 *   形状不符的下行帧一律丢弃（busy 由页面超时兜底恢复，对齐旧端 setTimeout 语义）。
 * - 无自动重连：断线后手动点「连接」（对齐旧端连接 tab 语义）。
 *
 * ## dev 模式握手失败原文透出
 * dev 模式下 `/api/ws` 被 `dev.rs:97-130` 拦截：未带合法 token → 401 JSON
 * `{"error":"unauthorized"}`；token 正确 → 503 JSON `{"error": "开发模式仅运行管理台与
 * 真实管理 API…", "error_code":"dev_inference_disabled", …}`。浏览器 WebSocket API
 * **读不到握手失败响应体**，故 `probeTestbenchHandshakeDetail()` 用普通 GET 打同一
 * 路径借道该 guard（拦截先于 WS 路由）拿回 JSON 原文；非 dev 下该 GET 得到 400
 * （无 JSON），按 null 处理回退通用文案。错误原文如实透出，不伪装成网络错误。
 */

import { getApiBaseUrl, getAuthToken } from "@/lib/api";

// ============================================================================
// W14b-1 扩展：server WS 增加 asr_test / Stt / 二进制中转；新增本地桥客户端。
// ============================================================================

/** 本地音频桥 init script 注入的常量（src-tauri/src/bridge.rs bridge_init_script）。 */
declare global {
  interface Window {
    __XIAOZHI_BRIDGE_PORT__?: number;
    __XIAOZHI_BRIDGE_TOKEN__?: string;
  }
}

/** `stt` 结果帧（protocol.rs ServerMessage::Stt：{session_id, text}）。 */
export interface SttWire {
  readonly text: string;
}

/** asr_test 上行：{type:"asr_test", action:"start"|"stop"}（ClientMessage::AsrTest，bench.rs:29-80 坐实字段名 action）。 */
export type AsrTestAction = "start" | "stop";

// ============================================================================
// 本地音频桥（Tauri Rust loopback WS；控制协议见 src-tauri/src/audio.rs BridgeCommand/BridgeEvent）
// ============================================================================

/** 本地桥事件帧（audio.rs BridgeEvent → JSON；Opus 帧走二进制不走这里）。 */
export type BridgeFrame =
  | { type: "started" }
  | { type: "recorded"; seconds: number; buckets: number[] }
  | { type: "wave"; seconds: number; buckets: number[] }
  | { type: "play_state"; playing: boolean; position: number; duration: number }
  | { type: "error"; code: string; message: string };

/** 读取 init script 注入的桥凭据；未注入（Web 环境 / 桥启动失败）返回 null。 */
export function bridgeInfo(): { port: number; token: string } | null {
  if (typeof window === "undefined") {
    return null;
  }
  const port = window.__XIAOZHI_BRIDGE_PORT__;
  const token = window.__XIAOZHI_BRIDGE_TOKEN__;
  if (typeof port !== "number" || port <= 0 || typeof token !== "string" || token === "") {
    return null;
  }
  return { port, token };
}

/** 本地桥 WS 地址（loopback；bridge_token 精确匹配）。 */
export function buildBridgeWsUrl(): string | null {
  const info = bridgeInfo();
  if (info === null) {
    return null;
  }
  return `ws://127.0.0.1:${info.port}/bridge?bridge_token=${encodeURIComponent(info.token)}`;
}

export interface BridgeSocketHandlers {
  /** 桥连接建立（可发 start_capture）。 */
  readonly onOpen: () => void;
  /** 桥连接断开（含首次连接失败；detail 为 close code）。 */
  readonly onClose: (code: number) => void;
  /** 文本事件帧（形状不符的帧已在本层丢弃）。 */
  readonly onFrame: (frame: BridgeFrame) => void;
  /** Opus 二进制帧（60ms 裸帧，前端原样中转到 server /api/ws）。 */
  readonly onBinary: (data: ArrayBuffer) => void;
}

/**
 * 本地音频桥 WS 客户端（一条连接；断开不自动重连，下次录音动作重建）。
 * 发送侧只有控制文本帧——Opus 由 Rust → 前端单向流出。
 */
export class BridgeSocket {
  private ws: WebSocket | null = null;
  private manualClose = false;

  public constructor(private readonly handlers: BridgeSocketHandlers) {}

  /** 发起连接；URL 由 buildBridgeWsUrl() 推导（null = 桥未注入，调用方自行提示）。 */
  public open(): boolean {
    const url = buildBridgeWsUrl();
    if (url === null) {
      return false;
    }
    const ws = new WebSocket(url);
    ws.binaryType = "arraybuffer";
    this.ws = ws;
    ws.onopen = () => {
      this.handlers.onOpen();
    };
    ws.onmessage = (event: MessageEvent<string | ArrayBuffer>) => {
      if (typeof event.data === "string") {
        let parsed: unknown;
        try {
          parsed = JSON.parse(event.data);
        } catch {
          return; // 非 JSON 文本帧丢弃
        }
        if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
          return;
        }
        const type = (parsed as Record<string, unknown>)["type"];
        if (typeof type !== "string") {
          return;
        }
        const record = parsed as Record<string, unknown>;
        switch (type) {
          case "started":
            this.handlers.onFrame({ type: "started" });
            break;
          case "recorded":
          case "wave":
            if (typeof record["seconds"] === "number" && Array.isArray(record["buckets"])) {
              const buckets = record["buckets"].filter(
                (b): b is number => typeof b === "number" && Number.isFinite(b),
              );
              if (type === "wave") {
                this.handlers.onFrame({ type: "wave", seconds: record["seconds"], buckets });
              } else {
                this.handlers.onFrame({ type: "recorded", seconds: record["seconds"], buckets });
              }
            }
            break;
          case "play_state":
            if (
              typeof record["playing"] === "boolean" &&
              typeof record["position"] === "number" &&
              typeof record["duration"] === "number"
            ) {
              this.handlers.onFrame({
                type: "play_state",
                playing: record["playing"],
                position: record["position"],
                duration: record["duration"],
              });
            }
            break;
          case "error":
            if (typeof record["code"] === "string" && typeof record["message"] === "string") {
              this.handlers.onFrame({
                type: "error",
                code: record["code"],
                message: record["message"],
              });
            }
            break;
          default:
            break; // 未知类型丢弃
        }
      } else if (event.data instanceof ArrayBuffer) {
        this.handlers.onBinary(event.data);
      }
    };
    ws.onclose = (event: CloseEvent) => {
      this.ws = null;
      if (!this.manualClose) {
        this.handlers.onClose(event.code);
      }
    };
    ws.onerror = () => undefined; // 详情由 onclose 报告
    return true;
  }

  public get isOpen(): boolean {
    return this.ws !== null && this.ws.readyState === WebSocket.OPEN;
  }

  public startCapture(): void {
    this.sendCommand("start_capture");
  }

  public stopCapture(): void {
    this.sendCommand("stop_capture");
  }

  public discardCapture(): void {
    this.sendCommand("discard_capture");
  }

  public playRecorded(): void {
    this.sendCommand("play_recorded");
  }

  public stopPlayback(): void {
    this.sendCommand("stop_playback");
  }

  /** 手动断开（不触发 onClose 回调）。 */
  public close(): void {
    this.manualClose = true;
    const ws = this.ws;
    this.ws = null;
    ws?.close();
  }

  private sendCommand(type: string): void {
    if (this.isOpen) {
      this.ws?.send(JSON.stringify({ type }));
    }
  }
}

/** 本地桥可用性说明（诚实呈现用）。 */
export const BRIDGE_UNAVAILABLE_HINT = "本地音频桥未就绪（需在 Tauri 桌面壳内运行；请重启应用）";

/** hello 帧（XiaoZhiModule.m:104-117 逐字段；本模块常量，连接建立后立即发送）。 */
const HELLO_FRAME: Record<string, unknown> = {
  type: "hello",
  version: 1,
  test: true,
  audio_params: {
    format: "opus",
    sample_rate: 16000,
    channels: 1,
    frame_duration: 60,
  },
};

/** `llm_test` 下行结果帧（字段与 protocol.rs LlmTestResult 一一对应，Option 缺省为 null）。 */
export interface LlmTestWire {
  readonly state: "ok" | "error";
  readonly text: string | null;
  readonly elapsedMs: number | null;
}

/** `nlu_test` 下行结果帧（字段与 protocol.rs NluTestResult 一一对应）。 */
export interface NluTestWire {
  readonly state: "ok" | "error";
  readonly intent: string | null;
  readonly confidence: number | null;
  readonly tokenizeMs: number | null;
  readonly inferMs: number | null;
  readonly text: string | null;
}

/** 形状守卫：JSON.parse 结果是否为普通对象。 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 可选字符串字段：缺失/非串按 null（不猜）。 */
function optionalString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

/** 可选数字字段：缺失/NaN/Infinity 按 null。 */
function optionalNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/** state 字段收窄：只认 "ok" | "error"，其余丢弃整帧（fail-closed，超时兜底会恢复）。 */
function testState(value: unknown): "ok" | "error" | null {
  return value === "ok" || value === "error" ? value : null;
}

/**
 * 计算 `/api/ws` 的绝对 http(s) URL（探针 fetch 用；token 非空拼 `?token=`）。
 * 无 window 环境（SSR/静态测试）返回 null——WebSocket 本就不在该环境构造。
 */
function httpEndpointUrl(token: string | null): string | null {
  const base = getApiBaseUrl();
  let url: string;
  if (base.startsWith("http://") || base.startsWith("https://")) {
    const parsed = new URL(base);
    parsed.pathname = "/api/ws";
    parsed.search = "";
    parsed.hash = "";
    url = parsed.toString();
  } else if (typeof window !== "undefined") {
    url = `${window.location.protocol}//${window.location.host}/api/ws`;
  } else {
    return null;
  }
  const trimmed = token?.trim() ?? "";
  if (trimmed === "") {
    return url;
  }
  return `${url}?token=${encodeURIComponent(trimmed)}`;
}

/** 测试台 WS 地址：http→ws / https→wss；token 非空拼 `?token=`（服务端明确允许）。 */
export function buildTestbenchWsUrl(token: string | null): string | null {
  const httpUrl = httpEndpointUrl(token);
  if (httpUrl === null) {
    return null;
  }
  return httpUrl.startsWith("https://")
    ? `wss://${httpUrl.slice("https://".length)}`
    : `ws://${httpUrl.slice("http://".length)}`;
}

/**
 * dev 模式握手失败原因探针：GET 打同一 `/api/ws` 端点借道 dev guard 拿 JSON 原文
 * （见文件头注释）。返回可读 detail（服务端原文），拿不到（非 dev / 网络 / 非 JSON）
 * 返回 null。**原文透出，不伪装成网络错误。**
 */
export async function probeTestbenchHandshakeDetail(token: string | null): Promise<string | null> {
  const probeUrl = httpEndpointUrl(token);
  if (probeUrl === null) {
    return null;
  }
  try {
    const response = await fetch(probeUrl, { headers: { Accept: "application/json" } });
    const text = await response.text();
    if (text.trim() === "") {
      return null;
    }
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch {
      return null; // 非 JSON（如非 dev 下 400 纯文本）：回退通用文案
    }
    if (!isRecord(parsed)) {
      return null;
    }
    const message = optionalString(parsed["error"]);
    const code = optionalString(parsed["error_code"]);
    if (message === null && code === null) {
      return null;
    }
    if (message !== null && code !== null) {
      return `${message}（error_code: ${code}）`;
    }
    return message ?? `error_code: ${code}`;
  } catch {
    return null;
  }
}

export interface TestbenchSocketHandlers {
  /** 连接建立且 hello 帧已发出（对齐旧端 connect 回调 success=true 时机）。 */
  readonly onOpen: () => void;
  /** 握手失败（从未 open 即 close；dev 原文由页面另调 probe 补充）。 */
  readonly onHandshakeFailure: () => void;
  /** 连接建立后意外断开（code 为 WebSocket close code）。 */
  readonly onUnexpectedClose: (code: number) => void;
  readonly onLlmTestResult: (result: LlmTestWire) => void;
  readonly onNluTestResult: (result: NluTestWire) => void;
  /** `stt` 结果帧（asr_test stop 的识别回包；W14b-1 起）。 */
  readonly onSttResult: (result: SttWire) => void;
}

/**
 * 一条测试台 WS 连接（每次「连接」新建实例；旧端 XiaoZhiModule.connect 同语义）。
 * 不自动重连；`close()` 是手动断开，不触发任何回调。
 */
export class TestbenchSocket {
  private ws: WebSocket | null = null;
  private opened = false;
  private manualClose = false;

  public constructor(
    private readonly url: string,
    private readonly handlers: TestbenchSocketHandlers,
  ) {}

  /** 发起连接；open 后自动发送 hello 帧，再回调 onOpen。 */
  public open(): void {
    const ws = new WebSocket(this.url);
    this.ws = ws;
    ws.onopen = () => {
      this.opened = true;
      ws.send(JSON.stringify(HELLO_FRAME));
      this.handlers.onOpen();
    };
    ws.onmessage = (event: MessageEvent<string>) => {
      this.routeMessage(event.data);
    };
    // 浏览器不提供握手失败详情，统一由 onclose 判定后回调（dev 原文走探针）。
    ws.onerror = () => undefined;
    ws.onclose = (event: CloseEvent) => {
      this.ws = null;
      if (this.manualClose) {
        return;
      }
      if (!this.opened) {
        this.handlers.onHandshakeFailure();
      } else {
        this.handlers.onUnexpectedClose(event.code);
      }
    };
  }

  /** 连接是否可用（OPEN）。 */
  public get isOpen(): boolean {
    return this.ws !== null && this.ws.readyState === WebSocket.OPEN;
  }

  /** 发 `llm_test`；连接不可用返回 false（调用方给「请先连接」提示）。 */
  public sendLlmTest(text: string): boolean {
    return this.sendTest("llm_test", text);
  }

  /** 发 `nlu_test`；连接不可用返回 false。 */
  public sendNluTest(text: string): boolean {
    return this.sendTest("nlu_test", text);
  }

  /**
   * 发 `asr_test`（action: start 开始缓冲 / stop 停止并整段识别；
   * 字段名 `action` 由 bench.rs:29-80 `handle_asr_test` 坐实）。
   */
  public sendAsrTest(action: AsrTestAction): boolean {
    if (!this.isOpen) {
      return false;
    }
    this.ws?.send(JSON.stringify({ type: "asr_test", action }));
    return true;
  }

  /**
   * 中转 Opus 二进制帧到 server `/api/ws`（hello version:1 → unwrap_uplink V1 裸帧）。
   * **绝对禁止开第二条 `/api/ws`**：test_buf 是会话级缓冲，双 WS = 缓冲分裂。
   */
  public sendBinary(data: ArrayBuffer): boolean {
    if (!this.isOpen) {
      return false;
    }
    this.ws?.send(data);
    return true;
  }

  /** 手动断开（对齐旧端 disconnect：「已断开」由页面回写状态）。 */
  public close(): void {
    this.manualClose = true;
    const ws = this.ws;
    this.ws = null;
    ws?.close();
  }

  private sendTest(type: "llm_test" | "nlu_test", text: string): boolean {
    if (!this.isOpen) {
      return false;
    }
    this.ws?.send(JSON.stringify({ type, text }));
    return true;
  }

  /** 下行文本帧路由：只认 llm_test / nlu_test 结果帧；其余（hello/stt/tts/…）忽略。 */
  private routeMessage(raw: string): void {
    let parsed: unknown;
    try {
      parsed = JSON.parse(raw);
    } catch {
      return;
    }
    if (!isRecord(parsed)) {
      return;
    }
    const type = parsed["type"];
    // stt 帧无 state 字段（{session_id, text}），先于 state 守卫路由。
    if (type === "stt") {
      const text = optionalString(parsed["text"]);
      if (text !== null) {
        this.handlers.onSttResult({ text });
      }
      return;
    }
    const state = testState(parsed["state"]);
    if (state === null) {
      return; // 形状不符整帧丢弃（fail-closed；busy 由页面超时兜底恢复）
    }
    if (type === "llm_test") {
      this.handlers.onLlmTestResult({
        state,
        text: optionalString(parsed["text"]),
        elapsedMs: optionalNumber(parsed["elapsed_ms"]),
      });
    } else if (type === "nlu_test") {
      this.handlers.onNluTestResult({
        state,
        intent: optionalString(parsed["intent"]),
        confidence: optionalNumber(parsed["confidence"]),
        tokenizeMs: optionalNumber(parsed["tokenize_ms"]),
        inferMs: optionalNumber(parsed["infer_ms"]),
        text: optionalString(parsed["text"]),
      });
    }
  }
}

/**
 * 解析测试台连接用 token：输入框显式值优先；为空回退统一鉴权令牌
 * （`getAuthToken()`，URL `?token=` / sessionStorage 口径）——服务端仅在
 * `[server].expected_token` 非空时校验，两处都没有就不拼 token。
 */
export function resolveTestbenchToken(tokenInput: string): string | null {
  const explicit = tokenInput.trim();
  if (explicit !== "") {
    return explicit;
  }
  return getAuthToken();
}
