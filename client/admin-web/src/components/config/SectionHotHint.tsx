/**
 * 配置段顶部的生效档位提示。
 *
 * 档位与 `hot_hint` 原话**只**来自 `GET /api/config/schema` 的 `app_sections`
 * （服务端 `plugins/registry/mod.rs:298-316` 的 `APP_SECTION_HOT` 加
 * `HotReload::hint()` :267-273）。该段不在 `app_sections` 时（例如 `aiui` 属于
 * `fullchain` 能力、不在应用级清单里）直接返回 null，不自造第四种档位文案。
 */

import type { ReactElement } from "react";
import { Badge } from "@/components/ui/badge";
import { HOT_LABEL } from "@/components/config/SaveBar";
import { useConfigSchema } from "@/lib/useConfigSchema";

export interface SectionHotHintProps {
  /** 服务端 `app_sections[].id`，例如 `server` / `audio` / `playback`。 */
  sectionId: string;
}

export function SectionHotHint({ sectionId }: SectionHotHintProps): ReactElement | null {
  const { schema } = useConfigSchema();
  const entry = schema?.app_sections.find((section) => section.id === sectionId);

  if (entry === undefined) {
    return null;
  }

  return (
    <div className="flex flex-wrap items-center gap-2 rounded-md border border-amber-600/40 bg-amber-50 px-3 py-2">
      <Badge
        variant="outline"
        className="border-amber-600/40 bg-white text-amber-900"
      >
        {HOT_LABEL[entry.hot] ?? entry.hot}
      </Badge>
      <p className="text-sm font-medium text-amber-900">{entry.hot_hint}</p>
    </div>
  );
}
