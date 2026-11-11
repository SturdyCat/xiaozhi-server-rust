/**
 * 配置页：18 段全量工作区（流程节点 + 全局段的完整清单）。
 *
 * 编排主体已抽到 `components/config/ConfigWorkspace.tsx`（/config 与 /settings
 * 共用，按 sectionIds 白名单过滤视图）；本文件只是 /config 路由的薄壳。
 * 分区清单唯一来源是 `configSections.ts`（对齐 Kuikly `ConfigSections.kt`）。
 * draft / advancedMode Provider 不挂在本页：由 router.tsx 的 Provider layout
 * 提供，与 /settings 共享同一份草稿（旧端 AdminPage.kt:71-72 语义）。
 *
 * 各段迁移进度（W3–W10 已全部完成，见 ConfigWorkspace 内 SECTION_COMPONENTS）：
 * W4/C1 server / aiui / audio（playback 挂 audio 页尾）；W5/C2 vad / asr /
 * command / wakeup；W6/C3 llm / tts；W7/C4 nlu；W8/C5 soul / memory；
 * W9/C6 echomuse / tuya / mcp（只读）；W10/C7 ota（动作面板，0 配置字段）。
 */

import type { ReactElement } from "react";
import { ConfigWorkspace } from "@/components/config/ConfigWorkspace";
import { CONFIG_SECTIONS } from "@/components/config/configSections";

export function ConfigPage(): ReactElement {
  return (
    <ConfigWorkspace
      title="配置"
      description="服务端参数与热生效档位；保存按字段增量提交。"
      sectionIds={CONFIG_SECTIONS.map((section) => section.id)}
    />
  );
}
