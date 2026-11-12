/**
 * 配置页分区清单（唯一数据源）。
 *
 * 来源：`client/shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigSections.kt:28-50`
 * （Kuikly 版唯一权威）。顺序、标签、分组与 `order` 必须与它保持一致，禁止在 JSX
 * 里另起一套 id 清单。
 */

import { sortBy } from "es-toolkit";

export type ConfigSectionGroup = "pinned" | "voice" | "agent" | "service";

export interface ConfigSection {
  id: string;
  label: string;
  group: ConfigSectionGroup;
  order: number;
  /** 是否已完成新管理台迁移；C0 总览与预设、W4/C1 server / aiui / audio（playback 挂 audio 页尾）、W5/C2 vad / asr / command / wakeup、W6/C3 llm / tts、W7/C4 nlu、W8/C5 soul / memory、W9/C6 echomuse / tuya / mcp（只读）、W10/C7 ota（动作面板，0 配置字段）。 */
  migrated: boolean;
}

export const CONFIG_SECTIONS: ConfigSection[] = [
  { id: "overview", label: "能力总览", group: "pinned", order: 0, migrated: true },
  { id: "presets", label: "预设", group: "pinned", order: 5, migrated: true },
  { id: "vad", label: "VAD · 语音分段", group: "voice", order: 10, migrated: true },
  { id: "asr", label: "ASR · 语音识别", group: "voice", order: 20, migrated: true },
  { id: "nlu", label: "NLU · 意图与技能", group: "voice", order: 30, migrated: true },
  { id: "llm", label: "LLM · 对话生成", group: "voice", order: 40, migrated: true },
  { id: "tts", label: "TTS · 语音合成", group: "voice", order: 50, migrated: true },
  { id: "command", label: "结束与指令", group: "voice", order: 60, migrated: true },
  { id: "wakeup", label: "唤醒与问候", group: "voice", order: 70, migrated: true },
  { id: "soul", label: "人格", group: "agent", order: 80, migrated: true },
  { id: "memory", label: "记忆", group: "agent", order: 90, migrated: true },
  { id: "aiui", label: "AIUI 全链路模式", group: "service", order: 100, migrated: true },
  { id: "audio", label: "音频与播放", group: "service", order: 110, migrated: true },
  { id: "server", label: "服务", group: "service", order: 120, migrated: true },
  { id: "echomuse", label: "EchoMuse 接入", group: "service", order: 130, migrated: true },
  { id: "tuya", label: "Tuya 接入", group: "service", order: 135, migrated: true },
  { id: "mcp", label: "MCP 工具", group: "service", order: 140, migrated: true },
  { id: "ota", label: "固件与 OTA", group: "service", order: 150, migrated: true },
];

/** 左菜单分组；`pinned` 置顶，其余顺序与 Kotlin `groups` 一致。 */
export const SECTION_GROUPS: { id: ConfigSectionGroup; label: string }[] = [
  { id: "pinned", label: "总览" },
  { id: "voice", label: "语音主流程" },
  { id: "agent", label: "上下文增强" },
  { id: "service", label: "服务与接入" },
];

/** 按 id 查找分区；未知 id 返回 undefined。 */
export function findSection(id: string): ConfigSection | undefined {
  return CONFIG_SECTIONS.find((section) => section.id === id);
}

/** 取某组的分区，按 `order` 升序（sortBy 返回新数组，稳定排序，与原 sort 等价）。 */
export function sectionsOf(group: ConfigSectionGroup): ConfigSection[] {
  return sortBy(
    CONFIG_SECTIONS.filter((section) => section.group === group),
    [(section) => section.order],
  );
}
