/**
 * 导航描述符单一数据源：侧边栏与路由表共用，避免「能点到但没路由」或
 * 「有路由但入口隐藏」。`tauriOnly` 与现有 Kuikly `NavDescriptor.macOnly` 同构。
 */
export interface NavDescriptor {
  /** 路由相对路径（HashRouter 下对应 /#/{key}）。 */
  readonly key: string;
  readonly label: string;
  readonly description: string;
  /** 仅在 Tauri 桌面环境可见。 */
  readonly tauriOnly: boolean;
}

export const NAV_DESCRIPTORS: readonly NavDescriptor[] = [
  {
    key: "overview",
    label: "总览",
    description: "设备在线状态、会话统计与服务运行概况。",
    tauriOnly: false,
  },
  {
    key: "devices",
    label: "设备管理",
    description: "设备列表、绑定关系、在线状态与固件版本。",
    tauriOnly: false,
  },
  {
    key: "devicehub",
    label: "设备中心",
    description: "EchoMuse 与涂鸦设备的接入、运行面板与扩展工具。",
    tauriOnly: false,
  },
  {
    key: "config",
    label: "配置管理",
    description: "服务端参数与配置热生效档位。",
    tauriOnly: false,
  },
  {
    key: "settings",
    label: "全局配置",
    description:
      "部署级参数：能力总览、预设、AIUI 全链路、服务、Tuya 接入与 OTA；无流程节点，与语音流程共用一份表单。",
    tauriOnly: false,
  },
  {
    key: "echomuse",
    label: "EchoMuse",
    description: "官方 /em/ 页面后续以 iframe 方式接入。",
    tauriOnly: false,
  },
  {
    key: "console",
    label: "控制台",
    description: "实时日志、WebSocket 会话与调试信息。",
    tauriOnly: false,
  },
  {
    key: "files",
    label: "文件管理",
    description: "固件、音频与静态资源的上传和管理。",
    tauriOnly: false,
  },
  {
    key: "plugins",
    label: "插件",
    description: "WASM 插件上传、生命周期管理与动态运行页。",
    tauriOnly: false,
  },
  {
    key: "testbench",
    label: "硬件测试台",
    description:
      "连接服务端测试 LLM 对话与 NLU 意图；录音识别与 TTS 合成待硬件桥接（仅 Tauri 桌面环境）。",
    tauriOnly: true,
  },
];

/** 按运行环境过滤导航项；路由器与侧边栏都必须使用此函数。 */
export function visibleNavDescriptors(tauriEnv: boolean): NavDescriptor[] {
  return NAV_DESCRIPTORS.filter((item) => !item.tauriOnly || tauriEnv);
}
