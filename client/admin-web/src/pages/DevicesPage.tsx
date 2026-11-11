import {
  type FormEvent,
  type ReactElement,
  useState,
} from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
} from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { formatDateTime, formatRelative } from "@/lib/utils";
import { useApiQuery } from "@/lib/useApiQuery";
import {
  getDevice,
  getDevices,
  injectDevice,
  type DeviceDetailInfo,
  type DeviceInfo,
  type DeviceSession,
} from "@/lib/adminApi";

/** 设备行：展示摘要，点击标题展开/收起详情。 */
function DeviceRow({
  device,
  expanded,
  onToggle,
}: {
  device: DeviceInfo;
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={onToggle}
              className="font-mono text-sm font-medium hover:underline"
            >
              {device.device_id}
            </button>
            <Badge
              className={
                device.online
                  ? "bg-emerald-500/15 text-emerald-700"
                  : "bg-gray-500/15 text-gray-600"
              }
            >
              {device.online ? "在线" : "离线"}
            </Badge>
            {device.is_test ? <Badge variant="outline">测试</Badge> : null}
            {device.mcp ? <Badge variant="outline">MCP</Badge> : null}
          </div>
          <span className="text-xs text-muted-foreground">
            最近活跃：{formatRelative(device.last_seen_ms)}
          </span>
        </div>
        <CardDescription className="text-xs">
          {device.board} · {device.firmware_version} · client {device.client_id}
        </CardDescription>
      </CardHeader>
      {expanded ? (
        <CardContent className="space-y-4 text-sm">
          <DeviceDetail deviceId={device.device_id} />
        </CardContent>
      ) : null}
    </Card>
  );
}

/** 设备详情：拉取单设备并展示会话/对话摘要 + 消息注入。 */
function DeviceDetail({ deviceId }: { deviceId: string }) {
  const query = useApiQuery(() => getDevice(deviceId), []);

  return (
    <div className="space-y-4">
      {query.error !== null ? (
        <p className="text-sm text-destructive">加载详情失败：{query.error}</p>
      ) : query.loading ? (
        <p className="text-sm text-muted-foreground">加载详情中…</p>
      ) : (
        <DeviceDetailBody detail={query.data} />
      )}
      <InjectForm deviceId={deviceId} onInjected={() => query.refetch()} />
    </div>
  );
}

/** 会话与对话摘要（data 可能为空，统一兜底为「无 … 数据」）。 */
function DeviceDetailBody({ detail }: { detail: DeviceDetailInfo | null }) {
  if (!detail) {
    return <p className="text-sm text-muted-foreground">无详情数据</p>;
  }

  return (
    <div className="space-y-4">
      <div>
        <div className="mb-2 font-medium">会话（{detail.sessions.length}）</div>
        {detail.sessions.length > 0 ? (
          <ul className="space-y-2">
            {detail.sessions.map((s: DeviceSession) => (
              <li key={s.session_id} className="rounded-md border px-3 py-2">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="font-mono text-xs">{s.session_id}</span>
                  <span className="text-xs text-muted-foreground">
                    {s.phase_display}
                  </span>
                </div>
                <div className="text-xs text-muted-foreground">
                  轮次 {s.turn_count}
                  {s.last_user_text ? ` · 最近：${s.last_user_text}` : ""}
                </div>
                {s.last_error ? (
                  <div
                    className="truncate text-xs text-destructive"
                    title={s.last_error}
                  >
                    {s.last_error}
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-muted-foreground">无会话数据</p>
        )}
      </div>

      <div>
        <div className="mb-2 font-medium">
          对话（{detail.conversations.length}）
        </div>
        {detail.conversations.length > 0 ? (
          <ul className="space-y-2">
            {detail.conversations.map((c) => (
              <li key={c.session_id} className="rounded-md border px-3 py-2">
                <div className="text-xs text-muted-foreground">
                  会话 {c.session_id} · 轮次 {c.turn_count}
                  {c.truncated ? " · 已截断" : ""}
                  {c.dropped ? " · 已丢弃" : ""}
                </div>
                {c.turns.length > 0 ? (
                  <div className="truncate text-xs">
                    {c.turns[c.turns.length - 1].role}：
                    {c.turns[c.turns.length - 1].text}
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-muted-foreground">无对话数据</p>
        )}
      </div>
    </div>
  );
}

/** 消息注入表单：行内成功/失败提示，不抛到顶层；成功后回调刷新详情。 */
function InjectForm({
  deviceId,
  onInjected,
}: {
  deviceId: string;
  onInjected: () => void;
}) {
  const [text, setText] = useState("");
  const [success, setSuccess] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = (event: FormEvent): void => {
    event.preventDefault();
    const value = text.trim();
    if (value === "") {
      return;
    }
    setSubmitting(true);
    setSuccess(null);
    setError(null);
    injectDevice(deviceId, value)
      .then((res) => {
        if (res.ok) {
          setSuccess(res.note ?? "注入成功");
          setText("");
          onInjected();
        } else {
          setError(res.message ?? "注入失败");
        }
      })
      .catch((err: unknown) => {
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        setSubmitting(false);
      });
  };

  return (
    <form onSubmit={handleSubmit} className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="输入要注入的消息（≤2000 字符）"
          maxLength={2000}
          className="min-w-[240px] flex-1 rounded-md border px-2 py-1 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
        />
        <Button type="submit" size="sm" disabled={submitting || text.trim() === ""}>
          {submitting ? "注入中…" : "注入"}
        </Button>
      </div>
      {success !== null ? (
        <p className="text-xs text-emerald-700">{success}</p>
      ) : null}
      {error !== null ? <p className="text-xs text-destructive">{error}</p> : null}
    </form>
  );
}

export function DevicesPage(): ReactElement {
  const devicesQuery = useApiQuery(getDevices);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const list = devicesQuery.data?.devices ?? [];
  const anyLoading = devicesQuery.loading;
  const firstError = devicesQuery.error;
  const lastUpdated = devicesQuery.updatedAt;

  const handleRefresh = (): void => {
    devicesQuery.refetch();
  };

  const toggle = (id: string): void => {
    setExpandedId(expandedId === id ? null : id);
  };

  return (
    <div className="mx-auto w-full max-w-6xl space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="space-y-1">
          <h2 className="text-2xl font-semibold tracking-tight">设备管理</h2>
          <p className="text-sm text-muted-foreground">
            设备列表、在线状态、会话与消息注入。
          </p>
        </div>
        <div className="flex items-center gap-3">
          <span className="text-xs text-muted-foreground">
            最后更新：{formatDateTime(lastUpdated)}
          </span>
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            disabled={anyLoading}
          >
            {anyLoading ? "刷新中…" : "刷新"}
          </Button>
        </div>
      </div>
      <Separator />

      {firstError !== null ? (
        <div className="flex items-center justify-between gap-3 rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          <span>加载失败：{firstError}</span>
          <Button variant="outline" size="sm" onClick={handleRefresh}>
            重试
          </Button>
        </div>
      ) : null}

      {anyLoading ? (
        <div className="text-sm text-muted-foreground">加载中…</div>
      ) : (
        <div className="space-y-3">
          {list.length > 0 ? (
            list.map((device) => (
              <DeviceRow
                key={device.device_id}
                device={device}
                expanded={expandedId === device.device_id}
                onToggle={() => toggle(device.device_id)}
              />
            ))
          ) : (
            <p className="text-sm text-muted-foreground">无设备数据</p>
          )}
        </div>
      )}
    </div>
  );
}
