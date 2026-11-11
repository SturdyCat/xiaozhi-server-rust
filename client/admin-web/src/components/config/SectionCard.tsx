/**
 * 配置段容器：标题 + 说明 + 内容 + 可选底栏，统一使用 shadcn Card。
 */

import type { ReactElement, ReactNode } from "react";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

export interface SectionCardProps {
  title: string;
  description?: string;
  footer?: ReactNode;
  children: ReactNode;
}

export function SectionCard({
  title,
  description,
  footer,
  children,
}: SectionCardProps): ReactElement {
  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        {description ? <CardDescription>{description}</CardDescription> : null}
      </CardHeader>
      <CardContent>{children}</CardContent>
      {footer ? <CardFooter>{footer}</CardFooter> : null}
    </Card>
  );
}
