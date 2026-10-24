//! `[echomuse]` 配置段（从 `mod.rs` 拆出：mod.rs 已近 600 行上限，配置自洽成文）。
//!
//! `[echomuse]` 属 **RestartOnly**：任何字段改动重启后才生效（含 `welcome` 文案）。

use serde::{Deserialize, Serialize};

/// 默认欢迎语：设备接入后立刻说一次，带最基本的使用提示（怎么开始对话）。
/// 换措辞直接改 `[echomuse].welcome`；留空 = 不说欢迎语。
pub const DEFAULT_WELCOME: &str = "我已连接，喊我的唤醒词或按一下按键就能开始对话。";

/// `[echomuse]` 配置段：EchoMuse 设备入口开关与凭据。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EchomuseConfig {
    /// 总开关。默认 **true**：EchoMuse 设备入口是「开机即用」的标准能力（mDNS 广播 +
    /// `/em/*` 路由就绪，设备刷机后即连）；不用 EchoMuse 设备的用户可显式改为 false——
    /// 关闭时不挂路由、不广播 mDNS，行为与没有这个入口完全一致。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 期望的设备 token（`X-Em-Token` 头）；为空字符串表示不校验。
    /// 对应 EchoMuse 设备凭据文件里的 token（配对时由 controller 下发）。
    #[serde(default)]
    pub token: String,
    /// 蓝牙代理总开关。默认 **false**：ack 不声明 `ble_connect`/`ble_adverts_data`、
    /// `0x06`/`0x08` 帧按未知忽略、管理面 BLE 端点 404——行为与关闭前完全一致。
    /// 开启后接入时向设备推送 `bleProxyEnabled`/`bleProxyConnections`（每设备
    /// settings 可覆盖），仅对 register 声明 `ble_connect` 的固件实际生效。
    #[serde(default)]
    pub ble_proxy: bool,
    /// 控制器专用端口（官方接口对齐）：>0 时**额外**在此端口监听设备面路由
    /// （`/em/*` + `/shell`），mDNS `plain_port` 也广播它——上游固件
    /// （`emos/init/init.c` 的 `tcp_peer()`）只把远端 8767/8770 的 ESTABLISHED
    /// 连接识别为 controller（串口 banner 的连接状态判据，全固件仅此一处按端口
    /// 判连接），设备拨官方端口才能被如实识别。默认 **8767**（官方明文
    /// controller 端口）；0 = 关闭（仅主端口）。主端口始终继续服务 `/em/*`
    /// （旧 endpoint 文件与联调脚本不受影响）；绑定失败只告警，mDNS 回退广播
    /// 主端口。见 [`crate::device::echomuse::listen`]。
    #[serde(default = "default_plain_port")]
    pub plain_port: u16,
    /// 设备接入欢迎语（`[echomuse]` RestartOnly）：EchoMuse 设备 register + data
    /// 认领、会话建立后**立刻**说一次（走唤醒问候语同一预制话术链路，首合成后经
    /// `hot-audio/` 落盘，稳态零合成）。默认带使用提示；**留空 = 不说**。
    /// 每次会话建立（含断线重连）都会说一遍——重连频繁时请自行取舍。
    #[serde(default = "default_welcome")]
    pub welcome: String,
    /// 音乐文件根目录（音乐面 `POST /devices/{id}/music` 的 `path` 必须落在
    /// 此目录内，防任意路径读取）。留空 = 音乐面 404。默认 `/data/music`
    /// （容器卷；`docker-compose.yml` 已可写挂载 `/data`）。
    #[serde(default = "default_music_dir")]
    pub music_dir: String,
}

/// `[echomuse].plain_port` 默认值：官方 controller 明文端口。
fn default_plain_port() -> u16 {
    8767
}

fn default_welcome() -> String {
    DEFAULT_WELCOME.to_string()
}

/// 音乐文件根目录默认值（容器 /data 卷下）。
fn default_music_dir() -> String {
    "/data/music".to_string()
}

fn default_true() -> bool {
    true
}

impl Default for EchomuseConfig {
    fn default() -> Self {
        EchomuseConfig {
            enabled: true,
            token: String::new(),
            ble_proxy: false,
            plain_port: default_plain_port(),
            welcome: default_welcome(),
            music_dir: default_music_dir(),
        }
    }
}

impl EchomuseConfig {
    /// 规范化（幂等；与各能力配置同口径：trim 一次）。
    pub fn normalize(&mut self) {
        self.token = self.token.trim().to_string();
        self.welcome = self.welcome.trim().to_string();
        self.music_dir = self.music_dir.trim().to_string();
    }
}

/// 会话启动欢迎语决策（纯函数）：仅 EchoMuse 承载的会话（`client_id == "echomuse"`，
/// 见 `routes.rs` 的 `run_session` 调用）且文案非空才返回 `Some`——xiaozhi WS 承载
/// 恒为 `None`，接入欢迎语义不外溢。
pub fn connect_welcome(client_id: &str, cfg: &EchomuseConfig) -> Option<String> {
    if client_id != "echomuse" {
        return None;
    }
    let text = cfg.welcome.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests;
