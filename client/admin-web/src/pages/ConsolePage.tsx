import { useCallback, useEffect, useRef, useState, type ReactElement } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { useToast } from "@/components/feedback/ToastHost";
import { getApiBaseUrl } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import { cn } from "@/lib/utils";

/** 日志环形缓冲上限；超过则丢弃最旧行，避免长时间运行撑爆内存。 */
const MAX_LINES = 2000;
/** 批量刷新间隔：日志高频到达时合并进 state，防止每帧重渲染风暴。 */
const FLUSH_INTERVAL_MS = 200;
/** 自动重连的初始退避间隔（毫秒）。 */
const RECONNECT_BASE_MS = 1000;
/** 自动重连的退避上限（毫秒）。 */
const RECONNECT_MAX_MS = 30000;

type ConnStatus = "connecting" | "open" | "closed" | "error";
type LevelFilter = "ALL" | "ERROR" | "WARN" | "INFO";

interface BacklogMessage {
  type: "backlog";
  lines: string[];
}
interface LogMessage {
  type: "log";
  line: string;
}

/**
 * 推导日志 WebSocket 地址。
 *
 * 注意：控制台日志走的是 `GET /api/logs`（tracing 镜像流），与管理台实时通道
 * `/api/admin/ws` 是两条独立 WS，因此不复用 `getWebSocketUrl()`。
 * 相对模式（默认 baseURL `/api`）→ `/api/logs`；绝对 baseURL 时把 http/https 换成 ws/wss。
 */
function buildLogsWsUrl(): string {
  const base = getApiBaseUrl();
  if (base.startsWith("http://") || base.startsWith("https://")) {
    return base.replace(/^http/, "ws") + "/logs";
  }
  if (typeof window === "undefined") {
    return `${base}/logs`;
  }
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${window.location.host}${base}/logs`;
}

/** 从一行日志中提取级别（行格式：`YYYY-MM-DD HH:MM:SS LEVEL [target] message`）。 */
function lineLevel(line: string): string | null {
  const match = /\s(ERROR|WARN|INFO|DEBUG|TRACE)\s/.exec(line);
  return match ? match[1] : null;
}

function statusMeta(status: ConnStatus): { label: string; className: string } {
  switch (status) {
    case "open":
      return { label: "已连接", className: "bg-emerald-500/15 text-emerald-700" };
    case "connecting":
      return { label: "连接中…", className: "bg-yellow-500/15 text-yellow-700" };
    case "error":
      return { label: "连接错误", className: "bg-red-500/15 text-red-700" };
    default:
      return { label: "已断开", className: "bg-gray-500/15 text-gray-600" };
  }
}

export function ConsolePage(): ReactElement {
  const [lines, setLines] = useState<string[]>([]);
  const [status, setStatus] = useState<ConnStatus>("connecting");
  const [level, setLevel] = useState<LevelFilter>("ALL");
  const [autoScroll, setAutoScroll] = useState<boolean>(true);
  const [generation, setGeneration] = useState<number>(0);
  const { show } = useToast();

  const wsRef = useRef<WebSocket | null>(null);
  const bufferRef = useRef<string[]>([]);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  /** 自动重连的定时器句柄；卸载或重建连接时必须清掉，避免多连接并存。 */
  const retryRef = useRef<number | null>(null);
  /** 当前退避间隔（毫秒），open 成功后归位到初始值。 */
  const retryDelayRef = useRef<number>(RECONNECT_BASE_MS);

  // 建立 / 重建 WebSocket 连接；generation 变化时重连。
  useEffect(() => {
    setStatus("connecting");

    /** 排一次指数退避重连（1s 起、30s 封顶）。构造失败与 onclose 共用同一条路径。 */
    const scheduleRetry = (): void => {
      const delay = retryDelayRef.current;
      retryDelayRef.current = Math.min(delay * 2, RECONNECT_MAX_MS);
      retryRef.current = window.setTimeout(() => {
        retryRef.current = null;
        setGeneration((g) => g + 1);
      }, delay);
    };

    let ws: WebSocket | null = null;
    try {
      ws = new WebSocket(buildLogsWsUrl());
    } catch {
      // 构造阶段抛错（地址非法等）同样要排重连：否则页面永远停在「连接错误」，
      // 只能靠用户手动点「重连」。退避由 scheduleRetry 保证不会空转。
      setStatus("error");
      scheduleRetry();
    }

    if (ws !== null) {
      wsRef.current = ws;
      ws.onopen = () => {
        setStatus("open");
        // 连上即重置退避，避免一次长时间断线把后续重连拖成 30s。
        retryDelayRef.current = RECONNECT_BASE_MS;
      };
      ws.onerror = () => setStatus("error");
      ws.onclose = () => {
        // 浏览器在握手失败时也会触发 onclose，这里不能无条件覆盖成 "closed"，
        // 否则用户看不出「连接被拒」与「正常断开」的区别。
        setStatus((prev) => (prev === "error" ? "error" : "closed"));
        // 服务端重启 / 网络抖动后自动重连；手动「重连」按钮走 generation。
        scheduleRetry();
      };
      ws.onmessage = (event: MessageEvent) => {
        let msg: BacklogMessage | LogMessage | { type: string };
        try {
          msg = JSON.parse(event.data as string) as BacklogMessage | LogMessage | { type: string };
        } catch {
          return;
        }
        if (msg.type === "backlog") {
          bufferRef.current = [];
          setLines((msg as BacklogMessage).lines.slice(-MAX_LINES));
        } else if (msg.type === "log") {
          bufferRef.current.push((msg as LogMessage).line);
        }
      };
    }

    const socket = ws;
    return () => {
      if (retryRef.current !== null) {
        window.clearTimeout(retryRef.current);
        retryRef.current = null;
      }
      if (socket === null) {
        return;
      }
      socket.onopen = null;
      socket.onerror = null;
      socket.onclose = null;
      socket.onmessage = null;
      socket.close();
      if (wsRef.current === socket) {
        wsRef.current = null;
      }
    };
  }, [generation]);

  // 每 200ms 把缓冲日志合并进 lines，保留最后 MAX_LINES 行。
  useEffect(() => {
    const id = window.setInterval(() => {
      if (bufferRef.current.length === 0) {
        return;
      }
      const incoming = bufferRef.current;
      bufferRef.current = [];
      setLines((prev) => {
        const merged = prev.concat(incoming);
        return merged.length > MAX_LINES ? merged.slice(merged.length - MAX_LINES) : merged;
      });
    }, FLUSH_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, []);

  // 自动滚到底部（暂停时不动）。
  useEffect(() => {
    if (!autoScroll) {
      return;
    }
    const el = scrollRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }, [lines, autoScroll]);

  const handleClear = useCallback(() => {
    bufferRef.current = [];
    setLines([]);
  }, []);

  const handleCopy = useCallback(() => {
    void copyText(lines.join("\n"))
      .then((ok) => {
        show(
          ok ? "已复制全部日志" : "复制失败：浏览器阻止了剪贴板访问，请手动选中日志复制",
          ok ? "success" : "error",
        );
      })
      .catch(() => {
        // copyText 契约上不 reject；这里兜底，避免未处理 rejection 让用户拿不到任何反馈。
        show("复制失败：浏览器阻止了剪贴板访问，请手动选中日志复制", "error");
      });
  }, [lines, show]);

  const visibleLines = level === "ALL" ? lines : lines.filter((line) => lineLevel(line) === level);

  const meta = statusMeta(status);

  return (
    <div className="mx-auto w-full max-w-5xl space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="space-y-1">
          <h2 className="text-2xl font-semibold tracking-tight">控制台</h2>
          <p className="text-sm text-muted-foreground">服务端实时日志（tracing 镜像流）。</p>
        </div>
        <div className="flex items-center gap-2">
          <Badge className={cn("shrink-0", meta.className)}>{meta.label}</Badge>
          <Button variant="outline" size="sm" onClick={() => setGeneration((g) => g + 1)}>
            重连
          </Button>
        </div>
      </div>
      <Separator />

      <Card>
        <CardHeader className="pb-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="space-y-1">
              <CardTitle>实时日志</CardTitle>
              <CardDescription>
                共 {lines.length} 行{level === "ALL" ? "" : `（筛选后 ${visibleLines.length} 行）`}
              </CardDescription>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <div className="flex items-center gap-1">
                {(["ALL", "ERROR", "WARN", "INFO"] as LevelFilter[]).map((lv) => (
                  <Button
                    key={lv}
                    variant={level === lv ? "default" : "outline"}
                    size="sm"
                    onClick={() => setLevel(lv)}
                  >
                    {lv}
                  </Button>
                ))}
              </div>
              <Button variant="outline" size="sm" onClick={() => setAutoScroll((v) => !v)}>
                {autoScroll ? "暂停滚动" : "继续滚动"}
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={handleCopy}
                disabled={lines.length === 0}
              >
                复制全部
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={handleClear}
                disabled={lines.length === 0}
              >
                清空
              </Button>
            </div>
          </div>
        </CardHeader>
        <CardContent>
          <div
            ref={scrollRef}
            className="h-[420px] overflow-auto rounded-md border bg-muted/40 p-3 font-mono text-xs leading-relaxed"
          >
            {visibleLines.length > 0 ? (
              visibleLines.map((line, index) => (
                <div key={index} className="whitespace-pre-wrap break-all">
                  {line}
                </div>
              ))
            ) : (
              <div className="text-muted-foreground">
                {lines.length === 0 ? "等待日志…" : "当前级别无匹配日志"}
              </div>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
