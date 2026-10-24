//! OWW 运行时资产分发：把 `silero_vad.onnx` 推进设备的唤醒词资产目录。
//!
//! ## 为什么需要这条链路
//!
//! 设备端语音门（`device/internal/client/speechgate.go`）优先用 Silero VAD 判断
//! "这一段是不是人声"；模型缺失时退化成**纯 RMS 门限**。现场实测的退化后果是
//! 死锁式的：24dB 增益后 `rms=0.0066`，而门限 `0.0634` 差了近 10 倍，`gate` 永远
//! 不开 → 一帧人声都不上行 → 服务端没有 ASR → `streamMic: no speech detected
//! within timeout — ending turn`（设备日志原话）。表现就是"按了键、说了话、没反应"。
//!
//! 上游把该文件随唤醒词资产一起下发（`em_oww_assets.VAD_NAME` + `em_oww_sync`）；本服务端
//! 用 [`install_vad`] 补上了「服务端→设备」这一段（唤醒词分类器仍只到数据卷，见
//! `docs/echomuse.md` 自定义唤醒词一节）。
//!
//! ## 资产来源与硬约束
//!
//! 模型**不进二进制**：跟 ASR/TTS 一样放在模型卷里（`$XIAOZHI_MODELS_DIR/oww/silero_vad.onnx`，
//! 默认 `/data/models/oww/silero_vad.onnx`）。下载由**服务端自己**在启动时做，且**只在
//! `[echomuse].enabled=true` 时**触发（见 [`spawn_prefetch`]）——没开 EchoMuse 的部署
//! 不会为它多下任何字节。这里负责「下载 → 读 → 必要时转 → 下发」。
//!
//! 来源与上游 `controller/Dockerfile` 的 `silero` 构建阶段一致：
//! **openWakeWord v0.5.1 release 里的 `silero_vad.onnx`**（不是 huggingface 镜像，
//! huggingface 那份虽是同一份二进制，但官方钉的凭证是 openWakeWord 这个 URL）。
//! 两端摘要都钉死：[`OFFICIAL_VAD_SHA256`] 是官方校验的**输入**，
//! [`TYPED_VAD_SHA256`] 是官方 `silero_typed.py` 重写后的**输出**（设备真正运行的那份）。
//!
//! 必须是 Silero VAD v5：输入 `input[1,N] + sr + h/c[2,1,64]`，设备按 640 样本一步、
//! 带 h/c 递归状态调用。
//!
//! 还必须是**typed-field 重写版**：原始副本的权重以 protobuf `raw_data` 存放，其在
//! 消息里的偏移不受对齐约束，armv7 上的 ORT 1.19 在 `CreateSession` 直接 SIGBUS（上游
//! `controller/tools/silero_typed.py` 就是干这件事）。下载源给的是原始版，所以这里在
//! 读取时调 [`super::onnx_typed`] 就地转换——转换与 Python 版逐比特等价，已经是 typed
//! 的文件则原样使用（幂等，不改写磁盘）。
//!
//! 环境变量 `XIAOZHI_OWW_VAD_PATH` 可指向替代文件（自建/更新模型时用）；无论
//! 走模型卷还是覆盖，都按**最终下发内容**算 sha256 让设备回算校验，装错文件不会
//! 静默生效。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::json;

use super::onnx_typed;
use super::{admin, shell};
use crate::runtime::Engines;

/// 设备端资产目录：必须与上游 `device/internal/wakeword/shadow/open.go` 的
/// `DefaultDir` 一致（`em_oww_assets.DEVICE_DIR` 同值，上游有测试钉住）。
const DEVICE_VAD_PATH: &str = "/data/local/share/echomuse/oww/silero_vad.onnx";
/// 固定命令：目录不存在就建（官方 reconcile 同样先 `mkdir -p`）。
const MKDIR: &str = "mkdir -p /data/local/share/echomuse/oww";

const VAD_NAME: &str = "silero_vad.onnx";
/// 模型根目录：与入口脚本的 `XIAOZHI_MODELS_DIR` 同源（容器内默认 `/data/models`）。
const ENV_MODELS_DIR: &str = "XIAOZHI_MODELS_DIR";
const DEFAULT_MODELS_DIR: &str = "/data/models";
/// 设备资产在模型卷下的相对路径。
///
/// **刻意不与** sherpa 的 `/data/models/silero_vad.onnx` 共用：那份是 sherpa-onnx 自己
/// 解析的原始模型（带 `raw_data` 也能跑），而设备要的是 typed 重写版，混用会把
/// SIGBUS 那份发给设备。
const VAD_RELATIVE: &str = "oww/silero_vad.onnx";
const ENV_OVERRIDE: &str = "XIAOZHI_OWW_VAD_PATH";

/// 官方来源。上游 `controller/Dockerfile` 的 `silero` 阶段就是从这里取的：
/// `https://github.com/dscripka/openWakeWord/releases/download/v0.5.1/silero_vad.onnx`。
/// 它是 Silero v5（`input[1,N] + sr + h/c[2,1,64]`），正是设备 `ort/vad.go` 要的签名。
const OFFICIAL_VAD_URL: &str =
    "https://github.com/dscripka/openWakeWord/releases/download/v0.5.1/silero_vad.onnx";
/// 官方钉的**输入**摘要（原始 `raw_data` 版）——上游拿它做 `sha256sum -c`。
const OFFICIAL_VAD_SHA256: &str =
    "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28";
/// 官方钉的**输出**摘要（`silero_typed.py` 重写后，设备真正运行的就是这份）。
/// 与本服务端 [`onnx_typed`] 的产物逐比特一致（已用真实模型核对）。
const TYPED_VAD_SHA256: &str = "da5e8a9c9c07981b907e8affe5497fe604958c51b67bff07231778d701d164d0";
/// 覆盖下载地址（内网镜像用）。与旧版入口脚本同名，部署侧无需改习惯。
const ENV_VAD_URL: &str = "OWW_VAD_URL";
/// GitHub 代理前缀。与入口脚本 `apply_proxy` 同语义：**只对 github.com 套用**，
/// `off`/空表示直连。部署环境（如 compose 默认值）不直连 GitHub 时，这里必须跟着走，
/// 否则「服务端自己下载」会比入口脚本那条路更脆。
const ENV_GITHUB_PROXY: &str = "GITHUB_PROXY";
/// 与入口脚本共用的自动下载总开关：`missing`（默认）/ `force` / `off`。
/// `off` 是部署方"别再联网下模型"的明确表态，这条链路必须一起遵守。
const ENV_AUTO_DOWNLOAD: &str = "XIAOZHI_AUTO_DOWNLOAD_MODELS";
/// 单个文件的下载上限：官方资产约 1.73 MiB，给 8 MiB 余量，防被错误页/大文件打爆内存。
const MAX_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) fn routes() -> axum::Router<Arc<Engines>> {
    use axum::routing::post;
    axum::Router::new().route(
        "/api/echomuse/devices/{id}/oww/install-vad",
        post(install_vad),
    )
}

/// 模型卷根目录（`XIAOZHI_MODELS_DIR`，空值等同未配置）。
fn models_dir() -> PathBuf {
    std::env::var(ENV_MODELS_DIR)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODELS_DIR))
}

/// `XIAOZHI_OWW_VAD_PATH` 的显式覆盖（空值等同未配置）。
fn override_path() -> Option<PathBuf> {
    std::env::var(ENV_OVERRIDE)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// 要下发的 VAD 文件：`XIAOZHI_OWW_VAD_PATH` 优先，否则模型卷下的标准位置。
fn vad_path() -> PathBuf {
    override_path().unwrap_or_else(|| models_dir().join(VAD_RELATIVE))
}

/// 资产是否已经可用。**0 字节也算缺失**——下载中断留下的空文件不能挡住补下。
fn is_ready(path: &std::path::Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    // 与下发时的校验共用同一实现，避免"下载算一种、下发算另一种"。
    shell::sha256_hex(bytes)
}

/// 下载到的字节属于哪一类（按官方钉的两段摘要识别）。
#[derive(Debug, PartialEq, Eq)]
enum VadSource {
    /// 官方**输出**摘要：已是 typed 布局，可以直接下发。
    Typed,
    /// 官方**输入**摘要：原始 `raw_data` 版，下发前必须重写。
    Raw,
    /// 不是官方那两个摘要（自建/镜像改过的文件）——需额外验证它是可解析的 ONNX。
    Unverified,
}

fn classify(digest: &str) -> VadSource {
    if digest.eq_ignore_ascii_case(TYPED_VAD_SHA256) {
        VadSource::Typed
    } else if digest.eq_ignore_ascii_case(OFFICIAL_VAD_SHA256) {
        VadSource::Raw
    } else {
        VadSource::Unverified
    }
}

/// 自动下载模式，语义与入口脚本的 `XIAOZHI_AUTO_DOWNLOAD_MODELS` 一致。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AutoDownload {
    Missing,
    Force,
    Off,
}

fn auto_download(value: Option<&str>) -> AutoDownload {
    match value.map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("off") => AutoDownload::Off,
        Some(value) if value.eq_ignore_ascii_case("force") => AutoDownload::Force,
        // 未设/空/未知一律按默认 `missing`：宁可补一次，也不要因为拼错而静默缺失。
        _ => AutoDownload::Missing,
    }
}

/// 模型卷里还没有设备语音门模型时，后台补一份。
///
/// **只由 `main` 在 `[echomuse].enabled=true` 时调用**：没开 EchoMuse 的部署不为它
/// 下载任何字节（与 ASR/TTS 的"无条件就绪"不同）。失败只告警，不阻断启动——
/// 缺了只影响 `POST …/oww/install-vad`，语音门本身仍能起（退化成 RMS 门限）。
pub(crate) fn spawn_prefetch() {
    if override_path().is_some() {
        tracing::debug!("XIAOZHI_OWW_VAD_PATH 已指定设备语音门模型，跳过自动下载");
        return;
    }
    let mode = auto_download(std::env::var(ENV_AUTO_DOWNLOAD).ok().as_deref());
    if mode == AutoDownload::Off {
        tracing::debug!(
            "{ENV_AUTO_DOWNLOAD}=off：不下载设备语音门模型（需自行预置模型卷里的 oww/silero_vad.onnx）"
        );
        return;
    }
    let path = vad_path();
    if mode == AutoDownload::Missing && is_ready(&path) {
        tracing::debug!(path = %path.display(), "设备语音门模型已就绪，跳过下载");
        return;
    }
    tokio::spawn(async move {
        match ensure_vad_model(mode == AutoDownload::Force).await {
            Ok(path) => tracing::info!(
                path = %path.display(),
                "设备语音门模型（Silero v5）已就绪：可由 POST /api/echomuse/devices/{{id}}/oww/install-vad 下发到设备"
            ),
            Err(error) => tracing::warn!(
                %error,
                "设备语音门模型下载失败；仅影响「安装 VAD」接口，ASR/TTS 不受影响"
            ),
        }
    });
}

/// 确保 `vad_path()` 指向一份可用模型：已就绪直接返回，否则按 [`OFFICIAL_VAD_URL`]
/// 下载、按官方摘要校验后原子落盘。`force` 为真时忽略已有文件重下。
async fn ensure_vad_model(force: bool) -> Result<PathBuf, String> {
    let path = vad_path();
    if !force && is_ready(&path) {
        return Ok(path);
    }
    let url = std::env::var(ENV_VAD_URL)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OFFICIAL_VAD_URL.to_owned());
    let bytes = download(&url).await?;
    let digest = sha256_hex(&bytes);
    match classify(&digest) {
        VadSource::Typed => tracing::debug!(%url, "下载到的是官方 typed 版（无需重写）"),
        VadSource::Raw => tracing::debug!(%url, "下载到官方原始版，下发前就地重写"),
        VadSource::Unverified => {
            // 自建/镜像改过的文件：至少得是能解析的 ONNX，否则绝不落盘（免得
            // 后面 install-vad 反复报"模型畸形"却看不出是下载坏了）。
            onnx_typed::rewrite(&bytes).map_err(|error| {
                format!(
                    "{url} 下载到的文件既不是官方摘要也不是可解析的 ONNX（sha256={digest}）：{error}"
                )
            })?;
            tracing::warn!(
                %url,
                %digest,
                "设备语音门模型不是官方钉的摘要，已接受但请确认来源（设备侧仍会按内容校验）"
            );
        }
    }
    write_atomic(&path, &bytes)?;
    tracing::info!(
        path = %path.display(),
        bytes = bytes.len(),
        %digest,
        %url,
        "设备语音门模型已下载"
    );
    Ok(path)
}

/// 按 `GITHUB_PROXY` 套代理（只对 github.com；`off`/空 = 直连），与入口脚本一致。
fn with_github_proxy(url: &str) -> String {
    let proxy = std::env::var(ENV_GITHUB_PROXY)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("off"));
    let is_github = url.starts_with("https://github.com/") || url.starts_with("http://github.com/");
    match proxy {
        Some(prefix) if is_github => format!("{prefix}{url}"),
        _ => url.to_owned(),
    }
}

async fn download(url: &str) -> Result<Vec<u8>, String> {
    let url = with_github_proxy(url);
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(DOWNLOAD_TIMEOUT)
        .user_agent(concat!("xiaozhi-server-rust/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败：{error}"))?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("请求 {url} 失败：{error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "{url} 返回 HTTP {status}（可换 OWW_VAD_URL 指内网镜像）"
        ));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "{url} 声称 {length} 字节，超过上限 {MAX_DOWNLOAD_BYTES}（疑似不是模型文件）"
            ));
        }
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取 {url} 响应失败：{error}"))?;
    if bytes.is_empty() {
        return Err(format!("{url} 返回空内容"));
    }
    if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
        return Err(format!(
            "{url} 返回 {} 字节，超过上限 {MAX_DOWNLOAD_BYTES}",
            bytes.len()
        ));
    }
    Ok(bytes.to_vec())
}

/// 先写 `.part` 再 rename：下载/校验中断不会留下"半个模型"被当成可用。
fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建目录 {} 失败：{error}", parent.display()))?;
    }
    let part = path.with_extension("onnx.part");
    std::fs::write(&part, bytes)
        .map_err(|error| format!("写入 {} 失败：{error}", part.display()))?;
    std::fs::rename(&part, path).map_err(|error| {
        let _ = std::fs::remove_file(&part);
        format!("落位 {} 失败：{error}", path.display())
    })
}

/// 读模型并保证是「typed 重写版」：原始 `raw_data` 版在 armv7 ORT 上会 SIGBUS，
/// 所以转换是下发前的**必要条件**，不是可选优化。
fn vad_asset() -> Result<Vec<u8>, String> {
    let path = vad_path();
    let bytes = std::fs::read(&path).map_err(|error| {
        format!(
            "VAD 模型不可读：{}（{error}）。服务端在 [echomuse].enabled=true 启动时会自动\
             从官方源下载到该路径（OWW_VAD_URL 可指内网镜像）；也可用 XIAOZHI_OWW_VAD_PATH \
             指向已下载的文件",
            path.display()
        )
    })?;
    let rewritten = onnx_typed::rewrite(&bytes)
        .map_err(|error| format!("VAD 模型不是可解析的 ONNX：{}（{error}）", path.display()))?;
    if rewritten.changed {
        // 下载源给的是原始 `raw_data` 版；这里已经就地转成 typed 布局（不改写磁盘）。
        tracing::debug!(
            path = %path.display(),
            bytes = rewritten.bytes.len(),
            "VAD 模型已转换为 typed 字段布局（armv7 ORT 需要）"
        );
    }
    Ok(rewritten.bytes)
}

/// `POST /api/echomuse/devices/{id}/oww/install-vad`
///
/// 同步执行（装完才回包）：先 `mkdir -p`，再经 shell 面把模型卷里的 VAD（必要时
/// 现场转成 typed 布局）推到 `/data/local/share/echomuse/oww/silero_vad.onnx`，设备侧
/// sha256 校验通过才落位。返回 `{"installed":true,"bytes":…,"sha256":…}`；模型缺失/
/// 畸形 500 `asset_unreadable`，离线/未批准/非 emOS 由 [`super::system::system_gate`]
/// 统一拒绝（409/403/422），传输失败按 [`shell::ShellError`] 映射（缺工具 502
/// `no_base64_decoder`/`no_hash_tool`，校验不符 502 `verify_failed`，且目标文件保持原样）。
///
/// 无需重启设备：设备每轮开轮都 `stat` 一次该文件（`openSilero` 注释："Cheap
/// when it is absent"），装好后**下一轮**的语音门就走 Silero 而不是 RMS。
async fn install_vad(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Err(response) = super::system::system_gate(&engines, &headers, &query, &id) {
        return response;
    }
    let asset = match vad_asset() {
        Ok(asset) => asset,
        Err(error) => {
            return admin::json(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": error, "code": "asset_unreadable"}),
            )
        }
    };
    if let Err(error) = shell::run(&id, MKDIR).await {
        return error.response();
    }
    match shell::stream_file(&id, &asset, DEVICE_VAD_PATH, 0o644).await {
        Ok(transfer) => admin::json(
            StatusCode::OK,
            json!({
                "device_id": id,
                "installed": true,
                "file": VAD_NAME,
                "path": DEVICE_VAD_PATH,
                "bytes": transfer.bytes,
                "sha256": transfer.sha256,
                // 设备在下一轮开轮时 stat 到该文件即生效，不需要重启进程。
                "applies": "next-turn",
            }),
        ),
        Err(error) => error.response(),
    }
}

#[cfg(test)]
#[path = "oww_tests.rs"]
mod tests;
