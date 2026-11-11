import type { ReactElement } from "react";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { getApiBaseUrl } from "@/lib/api";
import { isTauri } from "@/lib/platform";

export interface PlaceholderPageProps {
  readonly title: string;
  readonly description: string;
  readonly route: string;
  readonly origin: string;
  readonly notes?: readonly string[];
}

/** 8 个占位页共用的 shadcn 卡片骨架，保证每页都能真实点开与构建。 */
export function PlaceholderPage({
  title,
  description,
  route,
  origin,
  notes = [],
}: PlaceholderPageProps): ReactElement {
  const tauriEnv: boolean = isTauri();

  return (
    <div className="mx-auto w-full max-w-4xl space-y-6">
      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-3">
          <h2 className="text-2xl font-semibold tracking-tight">{title}</h2>
          <Badge variant="secondary">待迁移</Badge>
        </div>
        <p className="text-sm text-muted-foreground">{description}</p>
      </div>
      <Separator />
      <Card>
        <CardHeader>
          <CardTitle>页面占位</CardTitle>
          <CardDescription>本阶段只搭建开发环境，业务页面将在后续迁移中实现。</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4 text-sm">
          <dl className="grid grid-cols-[120px_1fr] gap-x-4 gap-y-3">
            <dt className="text-muted-foreground">路由</dt>
            <dd className="font-mono">{route}</dd>
            <dt className="text-muted-foreground">迁移来源</dt>
            <dd>{origin}</dd>
            <dt className="text-muted-foreground">API 入口</dt>
            <dd className="font-mono">@/lib/api → {getApiBaseUrl()}</dd>
          </dl>
          <Separator />
          <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
            <li>界面使用 shadcn/ui 组件与默认浅色主题。</li>
            {notes.map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
        </CardContent>
        <CardFooter className="flex flex-wrap items-center gap-2">
          <Badge variant="outline">
            {tauriEnv ? "运行环境：Tauri 桌面端" : "运行环境：Web 浏览器"}
          </Badge>
          <Badge variant="outline">状态：骨架占位</Badge>
        </CardFooter>
      </Card>
    </div>
  );
}
