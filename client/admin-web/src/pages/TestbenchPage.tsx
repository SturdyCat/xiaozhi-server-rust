import {
  Fragment,
  useCallback,
  useEffect,
  useState,
  type ReactElement,
} from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import {
  getCapabilities,
  isTauri,
  type AppCapabilities,
} from "@/lib/platform";

/**
 * 硬件测试台：仅 isTauri() 为真时由 router.tsx 注册。
 * 本阶段只验证「前端 → Tauri IPC 窄命令」通道，不实现任何硬件动作。
 */
export function TestbenchPage(): ReactElement {
  const [capabilities, setCapabilities] = useState<AppCapabilities | null>(null);
  const [error, setError] = useState<string>("");
  const [loading, setLoading] = useState<boolean>(false);

  const refresh = useCallback(async (): Promise<void> => {
    setLoading(true);
    setError("");
    try {
      const result = await getCapabilities();
      setCapabilities(result);
    } catch (cause: unknown) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const tauriEnv: boolean = isTauri();
  const rows: readonly (readonly [string, string])[] = [
    ["platform", capabilities?.platform ?? "—"],
    ["tauri", capabilities === null ? "—" : String(capabilities.tauri)],
    ["version", capabilities?.version ?? "—"],
  ];

  return (
    <div className="mx-auto w-full max-w-4xl space-y-6">
      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-3">
          <h2 className="text-2xl font-semibold tracking-tight">硬件测试台</h2>
          <Badge variant="secondary">仅 Tauri 环境</Badge>
        </div>
        <p className="text-sm text-muted-foreground">
          桌面壳能力检测与后续硬件命令的入口；Web 环境不挂载此路由。
        </p>
      </div>
      <Separator />
      <Card>
        <CardHeader>
          <CardTitle>桌面壳能力检测</CardTitle>
          <CardDescription>
            调用窄命令 app_capabilities，验证前端到 Tauri 的 IPC 通道。
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4 text-sm">
          {!tauriEnv ? (
            <p className="text-muted-foreground">
              当前为浏览器环境，本路由通常不会挂载；能力检测返回 Web 兜底值。
            </p>
          ) : null}
          <dl className="grid grid-cols-[120px_1fr] gap-x-4 gap-y-3">
            {rows.map(([label, value]) => (
              <Fragment key={label}>
                <dt className="text-muted-foreground">{label}</dt>
                <dd className="font-mono">{value}</dd>
              </Fragment>
            ))}
          </dl>
          {error !== "" ? (
            <p className="text-destructive">调用失败：{error}</p>
          ) : null}
          <Button
            variant="outline"
            size="sm"
            onClick={() => void refresh()}
            disabled={loading}
          >
            {loading ? "读取中…" : "重新读取"}
          </Button>
        </CardContent>
        <CardFooter className="flex flex-wrap items-center gap-2">
          <Badge variant="outline">
            麦克风 / Opus / 播放 / mDNS：本阶段未实现
          </Badge>
          <Badge variant="outline">接口可用 ≠ 真机通过</Badge>
        </CardFooter>
      </Card>
    </div>
  );
}
