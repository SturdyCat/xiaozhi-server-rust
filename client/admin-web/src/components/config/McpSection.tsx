/**
 * 「MCP 工具」段（W9 / Config C6）——`GET /api/mcp/tools` 的**只读表格**。
 *
 * ⚠️ mcp 不是配置段：本组件**不接 ConfigDraftContext、无 setField、不挂 SaveBar**，
 * 没有任何保存语义（对齐 `McpToolsCard.kt:40`「只读投影」口径）。呈现规则逐条复刻
 * 旧端 McpToolsCard.kt + McpToolsState.kt：
 * - 摘要行：加载失败「无法读取设备工具清单」/ 加载中「加载中…」/ 成功后计数（只含非零项）；
 * - 只在偏离时打标：在线不打标、正常工具不打标；「已断开」「指令闸门降级」「仅管理员」才出现；
 * - 失败显示原因并说清「下面是上次成功探测的结果」；空列表用服务端 note 原文解释
 *   （**空 ≠ 坏了**：设备空闲不建连）；
 * - description 原文显示（= 喂给大模型的提示词，解释「为什么 AI 会这么用」）。
 *
 * age_ms → 人话（「刚刚 / N 分钟前 / N 小时前 / N 天前」）：用服务端的**相对毫秒**，
 * 不用本地时钟——不把「3 分钟前」显示成「3 小时前」，也不把「上次探测」伪装成「现在」
 * （「已断开」徽标必须带相对时间）。
 *
 * 手动刷新，**无自动轮询**；错误行内显示不弹 toast（toast 只用于用户动作结果）。
 *
 * 新增行为声明（对齐 W8 口径，非旧端移植）：首次挂载自动加载一次（旧端 LoadableState
 * 同为挂载加载，行为一致）；刷新按钮在加载期间禁用（busy 门禁）。
 *
 * 端点契约（`mcpApi.ts` 头注释）：无条件基础路由（无门控、无鉴权、无参数），永远返回
 * JSON——不存在 SPA fallback HTML 200；空 devices 是正常态，不是「能力未启用」。
 */

import { useCallback, useEffect, useRef, useState, type ReactElement } from "react";
import { sumBy } from "es-toolkit";
import { LoaderCircle } from "lucide-react";
import { SectionCard } from "@/components/config/SectionCard";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { getMcpTools, mcpErrorMessage, type McpDevice, type McpTool } from "@/lib/mcpApi";

/** `age_ms` → 人话（用服务端相对毫秒，不用本地时钟；口径对齐 McpToolsState.kt:104-109）。 */
function ageText(ms: number): string {
  if (ms < 60_000) {
    return "刚刚";
  }
  if (ms < 3_600_000) {
    return `${Math.floor(ms / 60_000)} 分钟前`;
  }
  if (ms < 86_400_000) {
    return `${Math.floor(ms / 3_600_000)} 小时前`;
  }
  return `${Math.floor(ms / 86_400_000)} 天前`;
}

/** 指令闸门三态的展示文案（unknown/空 = 无需展示；McpToolsState.kt:160-165）。 */
function commandGateLabel(commandGate: string): string {
  if (commandGate === "compatible") {
    return "指令闸门兼容";
  }
  if (commandGate === "will_degrade") {
    return "说「退下」将断开";
  }
  return "";
}

/** 设备副标题：板型 · 固件 · 协议 · 探测时间（缺失项自动省略，老固件可能不报版本）。 */
function deviceSubtitle(device: McpDevice): string {
  const parts: string[] = [];
  if (device.board !== "") {
    parts.push(device.board);
  }
  if (device.firmwareVersion !== "") {
    parts.push(`固件 ${device.firmwareVersion}`);
  }
  if (device.protocolVersion !== "") {
    parts.push(`MCP ${device.protocolVersion}`);
  }
  parts.push(`探测于 ${ageText(device.ageMs)}`);
  return parts.join(" · ");
}

/** 工具构成一句话（只显示非零项；McpToolsState.kt:153-157）。 */
function toolSummary(device: McpDevice): string {
  let text = `${device.toolCount} 个工具`;
  if (device.userToolCount > 0) {
    text += `（${device.aiToolCount} 个可被 AI 调用 · ${device.userToolCount} 个仅管理员）`;
  }
  return text;
}

/** 一台设备：身份 + 在线状态 + 探测时间 + 工具清单（偏离才打标）。 */
function DeviceBlock({ device }: { device: McpDevice }): ReactElement {
  const gate = commandGateLabel(device.commandGate);
  return (
    <div className="space-y-1.5 border-t border-border pt-4 first:border-t-0 first:pt-0">
      <div className="flex flex-wrap items-center gap-2">
        <p className="text-sm font-semibold">
          {device.deviceId === "" ? "（设备未报 id）" : device.deviceId}
        </p>
        {/* 只在偏离时打标：在线是正常态不占视觉；「已断开」必须带相对时间——
            数据是上次探测，不把它伪装成「现在」。 */}
        {!device.online ? (
          <Badge variant="secondary">已断开（上次探测 {ageText(device.ageMs)}）</Badge>
        ) : null}
        {device.commandGate === "will_degrade" ? <Badge variant="destructive">{gate}</Badge> : null}
      </div>
      {device.commandGate === "compatible" ? (
        <p className="text-xs text-muted-foreground">{gate}（说「退下」会保持连接）</p>
      ) : null}
      <p className="text-xs text-muted-foreground">{deviceSubtitle(device)}</p>
      <p className="text-xs text-muted-foreground">{toolSummary(device)}</p>
      {device.error !== "" ? (
        <div className="rounded-md border border-amber-600/40 bg-amber-50 px-3 py-2 text-sm font-medium text-amber-900">
          最近一次探测未完成：{device.error}
          {device.tools.length > 0 ? "（下面显示的是上次成功探测的结果）" : ""}
        </div>
      ) : null}
      {device.tools.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          这台设备没有上报任何工具：可能是探测失败（见上），或固件未注册 MCP 工具（板级
          InitializeTools() 里没有 AddTool）。
        </p>
      ) : null}
      <div className="divide-y divide-border/60">
        {device.tools.map((tool) => (
          <ToolBlock key={tool.name} tool={tool} />
        ))}
      </div>
    </div>
  );
}

/** 一个工具：name（+ 仅管理员标记）/ 参数摘要 / description 原文（McpToolsCard.kt:142-170）。 */
function ToolBlock({ tool }: { tool: McpTool }): ReactElement {
  return (
    <div className="space-y-1 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <p className="text-sm font-medium">{tool.name}</p>
        {/* user_only 正是「为什么 AI 不会用这个工具」的答案，用警示色而非中性色。 */}
        {tool.userOnly ? <Badge variant="destructive">仅管理员</Badge> : null}
      </div>
      <p className="font-mono text-xs text-muted-foreground">{tool.signature}</p>
      {tool.description !== "" ? (
        <p className="text-xs leading-relaxed text-muted-foreground">{tool.description}</p>
      ) : null}
    </div>
  );
}

export function McpSection(): ReactElement {
  const mountedRef = useRef(true);
  const [data, setData] = useState<Awaited<ReturnType<typeof getMcpTools>> | null>(null);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  /** 手动读取；无自动轮询。错误行内显示，不弹 toast。 */
  const load = useCallback(async (): Promise<void> => {
    setLoading(true);
    try {
      const result = await getMcpTools();
      if (!mountedRef.current) {
        return;
      }
      setData(result);
      setError(null);
      setLoaded(true);
    } catch (err) {
      if (!mountedRef.current) {
        return;
      }
      setError(mcpErrorMessage(err));
      setLoaded(true);
    } finally {
      if (mountedRef.current) {
        setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 计数摘要（只含非零项，口径对齐 McpToolsState.kt:72-75）。
  const onlineCount = data === null ? 0 : data.devices.filter((d) => d.online).length;
  const toolTotal = data === null ? 0 : sumBy(data.devices, (d) => d.toolCount);
  const summaryParts: string[] = [];
  if (data !== null) {
    summaryParts.push(`共 ${data.devices.length} 台设备上报过工具`);
    if (toolTotal > 0) {
      summaryParts.push(`工具 ${toolTotal} 个`);
    }
    if (onlineCount > 0) {
      summaryParts.push(`${onlineCount} 台在线`);
    }
  }
  const summary =
    error !== null
      ? "无法读取设备工具清单"
      : loading && data === null
        ? "加载中…"
        : summaryParts.length > 0
          ? summaryParts.join(" · ")
          : "—";

  return (
    <SectionCard
      title="MCP 工具（ESP 设备上报）"
      description="只读投影：不参与配置保存，本段没有保存语义。工具清单只存在于固件里，服务器在会话开始时通过 MCP initialize + tools/list 拉取。"
    >
      <div className="space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-sm font-medium">{summary}</p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={loading}
            onClick={() => void load()}
          >
            {loading ? <LoaderCircle aria-hidden="true" className="size-4 animate-spin" /> : null}
            {loading ? "刷新中…" : "刷新"}
          </Button>
        </div>

        {error !== null ? (
          <div className="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm font-medium text-destructive">
            {error}（确认服务端地址可达后点「刷新」）
          </div>
        ) : null}

        {/* 空列表：展示服务端 note 原文——设备空闲不建连，空 ≠ 坏了。 */}
        {loaded && data !== null && data.devices.length === 0 ? (
          <p className="text-xs leading-relaxed text-muted-foreground">
            {data.note !== ""
              ? data.note
              : "还没有任何设备上报过 MCP 工具。设备在唤醒建立连接时才会被探测（OTA 不算），让设备唤醒一次后再点「刷新」。"}
          </p>
        ) : null}

        {data !== null &&
          data.devices.map((device) => <DeviceBlock key={device.deviceId} device={device} />)}
      </div>
    </SectionCard>
  );
}
