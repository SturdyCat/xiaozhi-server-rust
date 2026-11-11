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
    const state = testState(parsed["state"]);
    if (state === null) {
      return; // 形状不符整帧丢弃（fail-closed；busy 由页面超时兜底恢复）
    }
    const type = parsed["type"];
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
