/**
 * 全局配置页：Config 工作区的第二视图（方案 A 过滤视图）。
 *
 * 来源：Kuikly adminCore · ConfigWorkspaceViews.GlobalConfigLayout
 * （`ConfigWorkspaceViews.kt:224-230` 权威注释）——settings 没有语音工作流轨道、
 * 没有配置/测试 Tab，左侧全局段分组菜单、右侧复用同一套卡片、共用一份 draft、
 * 一次 `POST /api/config`；0 个 Settings 专属字段/端点/api。
 *
 * 「8 全局段」是过时口径，真实 6 段，全部已在 admin-web 实现并按白名单复用
 * ConfigWorkspace，本波 0 新表单、0 新端点、0 新 api 函数：
 * - 总览（旧端 ConfigSections.globalGroups 第一组）：overview（W3）/ presets（W3）
 * - 服务与接入（第二组）：aiui（W4）/ server（W4）/ tuya（W9，凭证 3 项刻意
 *   不渲染——唯一录入路径 devicehub `POST /api/tuya/credentials`）/ ota（W10，
 *   0 配置字段动作面板，不进 draft）
 *
 * 边界保持：一次 POST /api/config（部分更新 + expected_revision）；409 保留
 * draft 不自动重载；保存成功后回读 getConfig()；段级说明仍走服务端下发 schema。
 * 白名单外分区（如流程段）在能力总览「去配置」时回退 /config 首页（无深链参数）。
 */

import type { ReactElement } from "react";
import { ConfigWorkspace } from "@/components/config/ConfigWorkspace";

/** 全局段白名单（id 与旧端 ConfigSections.globalGroups 一致，顺序按菜单分组）。 */
const GLOBAL_SECTION_IDS: readonly string[] = [
  "overview",
  "presets",
  "aiui",
  "server",
  "tuya",
  "ota",
];

export function SettingsPage(): ReactElement {
  return (
    <ConfigWorkspace
      title="全局配置"
      description="部署级参数（能力总览、预设、AIUI 全链路、服务、Tuya 接入、OTA）——没有流程节点，与语音流程共用一份草稿、一次 POST /api/config。"
      sectionIds={GLOBAL_SECTION_IDS}
      jumpFallbackRoute="/config"
    />
  );
}
