/**
 * 「MCP 工具」段（W9 / Config C6）的数据源封装：`GET /api/mcp/tools`。
 *
 * 契约（`server/src/gateway/ws/mcp.rs` + `workflow/mcp/registry.rs:246-263` +
 * `workflow/mcp/discovery.rs:169-179`，均已逐行核实），**与 W6/W8 的端点有三点不同**：
 * 1. **无条件基础路由**：无 cfg 门控、无鉴权、无请求参数——永远返回 JSON，**不存在
 *    SPA fallback HTML 200**；本文件不写「HTML 200 → 能力未启用」的分支。
 * 2. **空 devices 是正常态**（设备空闲不建连，`registry.rs:259-261` note 原文）：
 *    不是错误、不是「能力未启用」，UI 必须用服务端 note 原文解释「为什么是空的」。
 * 3. **只读投影**：不参与 PUT /api/config，本段没有保存语义——本文件不接 draft、
 *    不提供任何写入方法。
 *
 * 错误纪律（与 soulMemoryApi 同风格）：判 `ok === true` 且 `devices` 是数组才算成功；
 * 形状不符（非对象 / ok!==true / devices 非数组）抛 [`McpUnavailableError`]；服务端
 * 出错是普通 JSON `{ok:false,...}`，读 message 透出。**不判 HTTP 200 为成功**。
 */

import { ApiError, apiGet } from "@/lib/api";
import type { JsonValue } from "@/lib/configApi";

/** MCP 工具清单读取失败（形状不符等结构性异常，UI 层用错误行呈现）。 */
export class McpUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "McpUnavailableError";
  }
}

/** 供页面判断是否走「无法读取设备工具清单」的摘要分支。 */
export function isMcpUnavailable(error: unknown): boolean {
  return error instanceof McpUnavailableError;
}

/** 把任意异常转成可展示的中文文案。 */
export function mcpErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

/** 判断是否为普通 JSON 对象（数组与 null 不算）。 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 尽力从错误响应体取 `{ok:false,message}` 的 message；纯文本兜底。 */
function readServerMessage(body: string): string | null {
  const text = body.trim();
  if (text === "") {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(text);
    if (isRecord(parsed)) {
      const message = parsed["message"];
      if (typeof message === "string" && message.trim() !== "") {
        return message.trim();
      }
    }
  } catch {
    // 非 JSON 错误体直接回落原文。
  }
  return text;
}

/** 错误归一：读服务端 message（`{ok:false,...}`），其余原样透传。 */
function normalizeError(error: unknown, action: string): never {
  if (error instanceof McpUnavailableError) {
    throw error;
  }
  if (error instanceof ApiError) {
    const detail = readServerMessage(error.body) ?? `HTTP ${error.status}`;
    throw new Error(`${action}失败：${detail}`);
  }
  if (error instanceof Error) {
    throw error;
  }
  throw new Error(String(error));
}

/** 形状守卫：`ok !== true` 或不是对象时抛出（不以 HTTP 200 判成功）。 */
function requireOk(raw: unknown, action: string): Record<string, unknown> {
  if (!isRecord(raw)) {
    throw new McpUnavailableError(`${action}：响应不是 JSON 对象。`);
  }
  if (raw["ok"] !== true) {
    const detail =
      typeof raw["message"] === "string" && raw["message"].trim() !== ""
        ? raw["message"].trim()
        : "服务端未给出失败原因";
    throw new Error(`${action}失败：${detail}`);
  }
  return raw;
}

// ============================================================
// GET /api/mcp/tools
// ============================================================

/** 一个设备上报的 MCP 工具（`McpToolInfo::to_json`，discovery.rs:169-179）。 */
export interface McpTool {
  name: string;
  /** 工具描述——**它同时是喂给大模型的提示词**，UI 必须显示原文。 */
  description: string;
  /** true = 固件 AddUserOnlyTool 注册：不出现在 tools/list、大模型看不到。 */
  userOnly: boolean;
  /** 参数摘要（服务端生成，如 `volume: integer（必填，0~100）`；无参数为「（无参数）」）。 */
  signature: string;
  /** 原始 inputSchema（unknown 透传；UI 只用 signature，不自行解析 JSON Schema）。 */
  inputSchema: JsonValue;
}

/** 一台设备的最近一次能力发现结果（`DeviceTools::to_json`，registry.rs:104-122）。 */
export interface McpDevice {
  deviceId: string;
  /** 板型名（固件 serverInfo.name，如 bread-compact-wifi）。 */
  board: string;
  firmwareVersion: string;
  protocolVersion: string;
  /** 最近一次探测时设备是否连着（会话结束会翻成 false）——「已断开」才打标。 */
  online: boolean;
  /** 距最近一次探测的毫秒数（服务端序列化时计算；UI 转人话，不用本地时钟）。 */
  ageMs: number;
  toolCount: number;
  /** 其中对大模型可见的工具数。 */
  aiToolCount: number;
  /** 其中 user_only 的数量。 */
  userToolCount: number;
  /** 指令闸门三态：compatible | will_degrade | unknown（P1-10）。 */
  commandGate: string;
  /** 探测未完成的原因（空 = 完整成功）。 */
  error: string;
  tools: McpTool[];
}

/** `GET /api/mcp/tools` 归一后的响应（只读投影，无保存语义）。 */
export interface McpTools {
  /** 最近探测的在前（服务端已排序）。**空数组是正常态**（设备空闲不建连）。 */
  devices: McpDevice[];
  count: number;
  /** 服务端说明原文（空列表时 UI 必须显示它，说清「不是坏了」）。 */
  note: string;
}

/** 安全读取字符串字段（缺失/类型不符返回空串）。 */
function str(raw: Record<string, unknown>, key: string): string {
  return typeof raw[key] === "string" ? (raw[key] as string) : "";
}

/** 安全读取数字字段（缺失/非有限数返回 0）。 */
function num(raw: Record<string, unknown>, key: string): number {
  const value = raw[key];
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

/**
 * 读取设备 MCP 工具清单。必须判 `ok === true && devices 是数组`，不以 200 判成功；
 * 形状不符抛 [`McpUnavailableError`]。**空 devices 是正常**，按成功返回。
 */
export async function getMcpTools(): Promise<McpTools> {
  const raw = await apiGet<unknown>("mcp/tools", {
    action: "读取设备工具清单",
    onError: normalizeError,
  });
  const body = requireOk(raw, "读取设备工具清单");
  if (!Array.isArray(body["devices"])) {
    throw new McpUnavailableError("读取设备工具清单：响应缺少 devices 数组（服务端返回异常）。");
  }
  const devices: McpDevice[] = [];
  for (const item of body["devices"]) {
    if (!isRecord(item)) {
      continue;
    }
    const toolsRaw = Array.isArray(item["tools"]) ? item["tools"] : [];
    const tools: McpTool[] = [];
    for (const toolRaw of toolsRaw) {
      if (!isRecord(toolRaw)) {
        continue;
      }
      tools.push({
        name: str(toolRaw, "name"),
        description: str(toolRaw, "description"),
        userOnly: toolRaw["user_only"] === true,
        signature: str(toolRaw, "signature"),
        inputSchema: (toolRaw["input_schema"] ?? null) as JsonValue,
      });
    }
    devices.push({
      deviceId: str(item, "device_id"),
      board: str(item, "board"),
      firmwareVersion: str(item, "firmware_version"),
      protocolVersion: str(item, "protocol_version"),
      online: item["online"] === true,
      ageMs: num(item, "age_ms"),
      toolCount: num(item, "tool_count"),
      aiToolCount: num(item, "ai_tool_count"),
      userToolCount: num(item, "user_tool_count"),
      commandGate: str(item, "command_gate"),
      error: str(item, "error"),
      tools,
    });
  }
  return {
    devices,
    count: num(body, "count"),
    note: str(body, "note"),
  };
}
