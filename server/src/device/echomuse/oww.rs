//! OWW 运行时资产分发：把 `silero_vad.onnx` + `libonnxruntime.so` 推进设备的唤醒词资产目录。
//!
//! ## 为什么需要这条链路
//!
//! 设备端语音门（`device/internal/client/speechgate.go`）优先用 Silero VAD 判断
//! "这一段是不是人声"；模型缺失时退化成**纯 RMS 门限**。现场实测的退化后果是
//! 死锁式的：24dB 增益后 `rms=0.0066`，而门限 `0.0634` 差了近 10 倍，`gate` 永远
//! 不开 → 一帧人声都不上行 → 服务端没有 ASR → `streamMic: no speech detected
//! within timeout — ending turn`（设备日志原话）。表现就是"按了键、说了话、没反应"。
//!
//! 而且是**两件套**：模型到位后设备会尝试 `dlopen` 同目录的 `libonnxruntime.so`——
//! 运行时缺失时同样退回 RMS（日志 `ort: open …/libonnxruntime.so: dlopen failed`）。
//! 官方从不把这两件打进固件（12.3MB 会翻倍 OTA 载荷与 A/B 槽位），而是随 OWW
//! 资产**带外安装**（`em_oww_assets`/`em_oww_sync`）：本模块对齐这条链路。
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
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

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

pub(super) const VAD_NAME: &str = "silero_vad.onnx";
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
/// ORT 运行时在设备上的路径（与 VAD 同目录，`speechgate.go` 的 dlopen 名）。
const DEVICE_ORT_PATH: &str = "/data/local/share/echomuse/oww/libonnxruntime.so";
/// ORT 运行时的设备侧文件名（资产清单的 key）。
pub(super) const ORT_SO_FILE: &str = "libonnxruntime.so";
/// ORT 运行时在模型卷下的相对路径（与 VAD 同级）。
const ORT_RELATIVE: &str = "oww/libonnxruntime.so";
/// 官方 ORT 来源：上游 `controller/Dockerfile` 就是取 maven central 的
/// `onnxruntime-android` AAR，解出 `jni/armeabi-v7a/libonnxruntime.so`（12.3MB），
/// **版本与 sha256 钉死**——官方原话：AAR 重编译不能静默改变装到设备上的东西。
const ORT_AAR_URL: &str = "https://repo1.maven.org/maven2/com/microsoft/onnxruntime/onnxruntime-android/1.19.2/onnxruntime-android-1.19.2.aar";
const ORT_AAR_SHA256: &str = "84737dd1fdb715f0ee695c0a9864f6949b66ecb4c46ca733bc9211bf0c2a7b5f";
/// AAR 内 armv7 运行时的固定路径。
const ORT_AAR_ENTRY: &str = "jni/armeabi-v7a/libonnxruntime.so";
/// 覆盖 ORT 下载地址（内网镜像用，如 maven.aliyun.com）。
const ENV_ORT_URL: &str = "OWW_ORT_URL";
/// AAR 下载上限：官方 AAR ≈ 16MB，给 32MB 余量（与 VAD 的 8MB 上限独立）。
const MAX_AAR_BYTES: u64 = 32 * 1024 * 1024;
/// 设备剩余空间下限（MB）：低于它拒绝推 ORT（官方 `FREE_FLOOR_MB` 同值）——
/// 设备装满会以"半个 .so + 一条不知所云的 dlopen 报错"收场，宁可明确跳过。
pub(super) const FREE_FLOOR_MB: u64 = 50;
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
    axum::Router::new()
        .route(
            "/api/echomuse/devices/{id}/oww/install-vad",
            post(install_vad),
        )
        // 资产五态只读接口在 [`super::oww_state`]（本文件已 700+ 行，不再加逻辑）。
        .merge(super::oww_state::routes())
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

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
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
    let mode = auto_download(std::env::var(ENV_AUTO_DOWNLOAD).ok().as_deref());
    if mode == AutoDownload::Off {
        tracing::debug!(
            "{ENV_AUTO_DOWNLOAD}=off：不下载设备语音门模型与运行时（需自行预置模型卷里的 oww/ 两件套）"
        );
        return;
    }
    // VAD 模型：显式覆盖路径只接管这一件（覆盖者自己负责文件内容与新鲜度）。
    if override_path().is_some() {
        tracing::debug!("XIAOZHI_OWW_VAD_PATH 已指定设备语音门模型，跳过自动下载");
    } else {
        let path = vad_path();
        if mode == AutoDownload::Missing && is_ready(&path) {
            tracing::debug!(path = %path.display(), "设备语音门模型已就绪，跳过下载");
        } else {
            tokio::spawn(async move {
                match ensure_vad_model(mode == AutoDownload::Force).await {
                    Ok(path) => tracing::info!(
                        path = %path.display(),
                        "设备语音门模型（Silero v5）已就绪：接入设备时将自动下发"
                    ),
                    Err(error) => tracing::warn!(
                        %error,
                        "设备语音门模型下载失败；设备语音门将退回 RMS 门限（说话可能无 ASR）"
                    ),
                }
            });
        }
    }
    // ORT 运行时：与 VAD 独立的资产、独立就绪检查——少了它，模型在也只会
    // dlopen 失败退回 RMS（这正是"模型装好了还报 RMS threshold"的那一环）。
    spawn_ort_prefetch(mode);
}

/// 确保 ORT 运行时就绪（后台）：官方钉版 AAR → 解出 armv7 `libonnxruntime.so` → 模型卷。
fn spawn_ort_prefetch(mode: AutoDownload) {
    let path = ort_path();
    if mode == AutoDownload::Missing && is_ready(&path) {
        tracing::debug!(path = %path.display(), "设备语音门运行时已就绪，跳过下载");
        return;
    }
    tokio::spawn(async move {
        match ensure_ort_model(mode == AutoDownload::Force).await {
            Ok(path) => tracing::info!(
                path = %path.display(),
                "设备语音门运行时（onnxruntime-android armv7）已就绪：接入设备时将自动下发"
            ),
            Err(error) => tracing::warn!(
                %error,
                "设备语音门运行时下载失败；模型装了也会因 dlopen 失败退回 RMS 门限"
            ),
        }
    });
}

/// ORT 运行时在模型卷的位置（`$XIAOZHI_MODELS_DIR/oww/libonnxruntime.so`）。
/// 刻意不受 `XIAOZHI_OWW_VAD_PATH` 影响：那是 VAD 文件自己的覆盖口。
fn ort_path() -> PathBuf {
    models_dir().join(ORT_RELATIVE)
}

/// 确保 `ort_path()` 指向官方钉版的 armv7 运行时：已就绪直接返回，否则下载 AAR、
/// 按官方摘要校验后解包落盘。AAR 摘要不匹配直接拒绝（运行时无法像 ONNX 那样
/// 做内容级验证，摘要就是唯一护栏——对齐上游 Dockerfile 的 `sha256sum -c`）。
async fn ensure_ort_model(force: bool) -> Result<PathBuf, String> {
    let path = ort_path();
    if !force && is_ready(&path) {
        return Ok(path);
    }
    let url = std::env::var(ENV_ORT_URL)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ORT_AAR_URL.to_owned());
    let aar = download_with_limit(&url, MAX_AAR_BYTES).await?;
    let digest = sha256_hex(&aar);
    if !digest.eq_ignore_ascii_case(ORT_AAR_SHA256) {
        return Err(format!(
            "{url} 的 sha256={digest} 与官方钉版不符（{ORT_AAR_SHA256}），拒绝解包落盘；\
             可用 {ENV_ORT_URL} 指向内网镜像"
        ));
    }
    let runtime = extract_armv7_runtime(&aar)?;
    if runtime.is_empty() {
        return Err(format!("{url} 里 {ORT_AAR_ENTRY} 为空"));
    }
    write_atomic(&path, &runtime)?;
    tracing::info!(
        path = %path.display(),
        bytes = runtime.len(),
        %url,
        "设备语音门运行时已下载（onnxruntime-android 1.19.2 armv7，摘要钉版 AAR 解出）"
    );
    Ok(path)
}

/// 从官方钉版 AAR（zip 容器）里解出 armv7 运行时。
fn extract_armv7_runtime(aar: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(aar))
        .map_err(|error| format!("AAR 不是有效的 zip 容器：{error}"))?;
    let mut entry = archive
        .by_name(ORT_AAR_ENTRY)
        .map_err(|error| format!("AAR 里找不到 {ORT_AAR_ENTRY}：{error}"))?;
    let mut out = Vec::with_capacity(entry.size() as usize);
    std::io::Read::read_to_end(&mut entry, &mut out)
        .map_err(|error| format!("解压 {ORT_AAR_ENTRY} 失败：{error}"))?;
    Ok(out)
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
    download_with_limit(url, MAX_DOWNLOAD_BYTES).await
}

/// 上限参数化的下载：VAD 模型 8MB、ORT AAR 32MB，各对各自的假想错误页设防。
async fn download_with_limit(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
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
        return Err(format!("{url} 返回 HTTP {status}"));
    }
    if let Some(length) = response.content_length() {
        if length > max_bytes {
            return Err(format!(
                "{url} 声称 {length} 字节，超过上限 {max_bytes}（疑似不是目标文件）"
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
    if bytes.len() as u64 > max_bytes {
        return Err(format!(
            "{url} 返回 {} 字节，超过上限 {max_bytes}",
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
    // 通用 `.part` 后缀（追加而非替换扩展名）：`silero_vad.onnx.part` /
    // `libonnxruntime.so.part`，天然互不干扰。
    let mut part_name = path
        .file_name()
        .map(|name| name.to_owned())
        .ok_or_else(|| format!("非法目标路径 {}", path.display()))?;
    part_name.push(".part");
    let part = path.with_file_name(part_name);
    std::fs::write(&part, bytes)
        .map_err(|error| format!("写入 {} 失败：{error}", part.display()))?;
    std::fs::rename(&part, path).map_err(|error| {
        let _ = std::fs::remove_file(&part);
        format!("落位 {} 失败：{error}", path.display())
    })
}

/// 读模型并保证是「typed 重写版」：原始 `raw_data` 版在 armv7 ORT 上会 SIGBUS，
/// 所以转换是下发前的**必要条件**，不是可选优化。
///
/// 结果按 `(文件 mtime, 文件长度)` 缓存（见 [`VAD_CACHE`]）：本函数的调用点是高频
/// **只读**的——管理台设备页轮询 `GET …/oww/state`、每次设备注册触发的
/// `ensure_on_device_assets`、以及 `install-vad` 都走它，而重写要解析 1.8MB protobuf
/// 并逐 tensor 重排字节，没必要每次重跑（日志里成串的"已转换为 typed 字段布局"
/// 就是缓存缺失的痕迹）。
pub(super) fn vad_asset() -> Result<Vec<u8>, String> {
    let path = vad_path();
    let meta = std::fs::metadata(&path).map_err(|error| unreadable(&path, &error))?;
    let len = meta.len();
    // mtime 取不到（个别文件系统/挂载）就退化为不缓存，宁可多算也不能给旧字节。
    let stamp = meta.modified().ok();
    let cache = VAD_CACHE.get_or_init(|| Mutex::new(None));
    if let Some(stamp) = stamp {
        if let Ok(guard) = cache.lock() {
            if let Some((cached_path, cached_stamp, cached_len, bytes)) = guard.as_ref() {
                if *cached_path == path && *cached_stamp == stamp && *cached_len == len {
                    return Ok(bytes.clone());
                }
            }
        }
    }
    let bytes = std::fs::read(&path).map_err(|error| unreadable(&path, &error))?;
    let rewritten = onnx_typed::rewrite(&bytes)
        .map_err(|error| format!("VAD 模型不是可解析的 ONNX：{}（{error}）", path.display()))?;
    if rewritten.changed {
        // 下载源给的是原始 `raw_data` 版；这里已经就地转成 typed 布局（不改写磁盘）。
        // 缓存命中后不会再走到这里，所以这条日志天然只在"首次读/换模型"时出现。
        tracing::debug!(
            path = %path.display(),
            bytes = rewritten.bytes.len(),
            "VAD 模型已转换为 typed 字段布局（armv7 ORT 需要）"
        );
    }
    if let Some(stamp) = stamp {
        if let Ok(mut guard) = cache.lock() {
            *guard = Some((path.clone(), stamp, len, rewritten.bytes.clone()));
        }
    }
    Ok(rewritten.bytes)
}

/// typed 重写结果缓存：`(路径, mtime, 文件长度) → 重写后字节`。
///
/// 失效键用 `(路径, mtime, len)` 而不是"进程内只算一次"：模型可能被重新下载、或被
/// `XIAOZHI_OWW_VAD_PATH` 改指到另一个文件，改完不重启也要能生效；路径必须入键，
/// 否则"换指到另一个文件但 mtime/长度巧合相同"会拿到上一份字节。
type VadCache = Mutex<Option<(PathBuf, SystemTime, u64, Vec<u8>)>>;
static VAD_CACHE: OnceLock<VadCache> = OnceLock::new();

/// 模型文件读不到时的统一说明（顺带给出"怎么补上"的出路）。
fn unreadable(path: &FsPath, error: &std::io::Error) -> String {
    format!(
        "VAD 模型不可读：{}（{error}）。服务端在 [echomuse].enabled=true 启动时会自动\
         从官方源下载到该路径（OWW_VAD_URL 可指内网镜像）；也可用 XIAOZHI_OWW_VAD_PATH \
         指向已下载的文件",
        path.display()
    )
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

/// 设备侧 OWW 资产清单（**一趟 shell**）：对齐官方 `em_oww_sync::device_state`——
/// 目录 glob 逐文件哈希 + `df -m /data` 剩余空间，一次往返问完（官方原话
/// "the ordinary path is one shell round trip"）。官方用 md5sum，这里用 sha256sum
/// （busybox 既有 applet，与下发校验同一算法）；`tail -1` 取 df 数据行，多出来的
/// 表头行不含 `%` 列、解析自然跳过。清单不含 mtime：那是官方给自定义分类器
/// LRU 淘汰用的，语音门两件套没有淘汰语义。
const DEVICE_STATE_CHECK: &str = "for f in /data/local/share/echomuse/oww/*.so /data/local/share/echomuse/oww/*.onnx; do [ -f \"$f\" ] && echo \"$(busybox sha256sum \"$f\" | busybox cut -d' ' -f1) $f\"; done; echo \"FREE $(busybox df -m /data | busybox tail -1)\"";

/// 设备侧 OWW 资产现状：文件名 → sha256，以及 /data 剩余 MB。
/// `pub(super)`：只读五态接口（[`super::oww_state`]）复用同一次清单读取。
pub(super) struct DeviceOwwState {
    pub(super) files: HashMap<String, String>,
    pub(super) free_mb: Option<u64>,
}

/// 一趟读设备侧现状。**读不到 ≠ 缺失**（官方"failure to LOOK is not evidence of
/// absence"）：返回 None 时调用方必须什么都不做——shell 面可能还没起来，刚接入
/// 的设备大概率如此，此时改设备才是错的。
pub(super) async fn device_state(id: &str) -> Option<DeviceOwwState> {
    let out = shell::run(id, DEVICE_STATE_CHECK).await.ok()?;
    let mut files = HashMap::new();
    let mut free_mb = None;
    for line in out.lines() {
        if let Some(row) = line.strip_prefix("FREE ") {
            free_mb = parse_free_mb(row);
            continue;
        }
        let mut fields = line.split_whitespace();
        let digest = fields.next()?;
        let path = fields.next()?;
        // 只认 64 位 sha256 行：`filter` 里带长度判定，省掉一层嵌套 if。
        if let Some(name) = path.rsplit('/').next().filter(|_| digest.len() == 64) {
            files.insert(name.to_owned(), digest.to_ascii_lowercase());
        }
    }
    Some(DeviceOwwState { files, free_mb })
}

/// 官方 `parse_free_mb` 同语义：busybox 会在长文件系统名时换行，列序号不稳定
/// （awk 按固定列取会把 "65%" 当成可用空间），以"以 % 结尾的使用率列"为锚，
/// 取它前面第一个数字字段。
fn parse_free_mb(df_row: &str) -> Option<u64> {
    let tokens: Vec<&str> = df_row.split_whitespace().collect();
    tokens.iter().enumerate().rev().find_map(|(i, field)| {
        field.strip_suffix('%')?.parse::<u8>().ok()?; // 只认"% 结尾"的使用率列
        let prev = *tokens.get(i.checked_sub(1)?)?;
        prev.parse::<u64>().ok()
    })
}

/// 设备接入后自动确保语音门**两件套**（模型 + 运行时）已就位。
///
/// 对齐官方 `em_oww_sync::reconcile`：OWW 资产**自动同步**到设备，不靠手动按钮。
/// 仅在已批准、在线、emOS 设备接入时由 [`super::routes::handle_control_ws`] 调用；
/// 结构与官方同构：**一趟读设备现状** → 逐件摘要比对（已装且匹配即跳过，不重传）
/// → 需要的才经受限 shell 面下发。两件独立同步：模型（1.8MB）到位而运行时
/// （12.3MB）缺失时，设备照样退回 RMS（dlopen 失败）。失败只告警、不阻断语音
/// 会话；下次接入自动重试。
///
/// 前置：服务端资产必须已就绪（`spawn_prefetch` 在 `[echomuse].enabled=true` 启动时
/// 下载；尚未就绪时跳过，待下次接入再试）。
pub(super) fn ensure_on_device_assets(id: String, base_os: Option<String>) {
    if !base_os
        .as_deref()
        .is_some_and(|os| os.eq_ignore_ascii_case("emos"))
    {
        return; // 仅 emOS 设备需要设备侧语音门资产（其它 base_os 由各自链路处理）
    }
    tokio::spawn(async move {
        let Some(state) = device_state(&id).await else {
            tracing::debug!(
                device_id = %id,
                "设备 OWW 资产清单读取失败（shell 面可能未就绪）——本次不动设备，待下次接入再试"
            );
            return;
        };
        sync_vad(&id, &state).await;
        sync_runtime(&id, &state).await;
    });
}

/// 同步设备侧语音门模型（`silero_vad.onnx`，typed 重写版）。
async fn sync_vad(id: &str, state: &DeviceOwwState) {
    if !is_ready(&vad_path()) {
        tracing::debug!(
            device_id = %id,
            "设备语音门模型尚未在服务端就绪，跳过自动下发（待 [echomuse].enabled 启动下载后，下次接入再尝试）"
        );
        return;
    }
    // 幂等：设备已装 typed 版则跳过（官方"已同步不重传"同语义）。
    if let Some(existing) = state.files.get(VAD_NAME) {
        if existing.eq_ignore_ascii_case(TYPED_VAD_SHA256) {
            tracing::debug!(device_id = %id, "设备语音门模型已就位，跳过自动下发");
            return;
        }
        tracing::info!(device_id = %id, "设备语音门模型版本不符，重新下发");
    }
    let asset = match vad_asset() {
        Ok(asset) => asset,
        Err(error) => {
            tracing::warn!(device_id = %id, %error, "设备语音门模型读取失败，跳过自动下发");
            return;
        }
    };
    if let Err(error) = shell::run(id, MKDIR).await {
        tracing::warn!(device_id = %id, ?error, "设备语音门模型下发前置 mkdir 失败，跳过");
        return;
    }
    match shell::stream_file(id, &asset, DEVICE_VAD_PATH, 0o644).await {
        Ok(transfer) => tracing::info!(
            device_id = %id,
            bytes = transfer.bytes,
            sha256 = %transfer.sha256,
            "设备语音门模型已自动下发（设备下一轮开轮即生效，无需重启）"
        ),
        Err(error) => tracing::warn!(
            device_id = %id,
            ?error,
            "设备语音门模型自动下发失败（可经管理台手动重试 install-vad）"
        ),
    }
}

/// 同步设备侧 ORT 运行时：speechgate 在找到 `silero_vad.onnx` 后会 `dlopen` 同目录的
/// `libonnxruntime.so`——运行时缺失时同样退回 RMS（且报错不知所云）。这是"模型装好
/// 了还报 RMS threshold"的最后一环。幂等 + 失败只告警，同 VAD。
async fn sync_runtime(id: &str, state: &DeviceOwwState) {
    let asset = match ort_asset() {
        Ok(asset) => asset,
        Err(error) => {
            tracing::debug!(device_id = %id, %error, "ORT 运行时未就绪，跳过下发（待启动下载完成后，下次接入再试）");
            return;
        }
    };
    let digest = sha256_hex(&asset);
    if let Some(existing) = state.files.get(ORT_SO_FILE) {
        if existing.eq_ignore_ascii_case(&digest) {
            tracing::debug!(device_id = %id, "ORT 运行时已就位，跳过下发");
            return;
        }
        tracing::info!(device_id = %id, "ORT 运行时版本不符，重新下发");
    }
    // 空间护栏（官方 FREE_FLOOR_MB 同值）：设备装满 → 半个 .so → dlopen 报错不知所云。
    // 官方语义：空间检查在算完"真正要推什么"之后——已齐的设备绝不被空间挡。
    if state.free_mb.is_some_and(|free| free < FREE_FLOOR_MB) {
        tracing::warn!(
            device_id = %id,
            free_mb = state.free_mb.unwrap_or_default(),
            need_mb = asset.len() / (1024 * 1024),
            "设备剩余空间不足，跳过 ORT 运行时下发（官方下限 {FREE_FLOOR_MB}MB）"
        );
        return;
    }
    if let Err(error) = shell::run(id, MKDIR).await {
        tracing::warn!(device_id = %id, ?error, "ORT 运行时下发前置 mkdir 失败，跳过");
        return;
    }
    match shell::stream_file(id, &asset, DEVICE_ORT_PATH, 0o644).await {
        Ok(transfer) => tracing::info!(
            device_id = %id,
            bytes = transfer.bytes,
            sha256 = %transfer.sha256,
            "ORT 运行时已自动下发（设备下一轮开轮即生效，语音门不再退回 RMS）"
        ),
        Err(error) => tracing::warn!(
            device_id = %id,
            ?error,
            "ORT 运行时自动下发失败（12.3MB 走 shell 面，弱网可能超时；下次设备接入自动重试）"
        ),
    }
}

/// ORT 运行时资产：读模型卷里由 `spawn_ort_prefetch` 下载的 armv7 so。
/// 摘要已在下载时钉过（AAR 级），落盘后只做"非空文件"检查——对自置文件不做
/// 内容级验证（无法验证），装错由设备 dlopen 失败兜底（退回 RMS，不崩）。
pub(super) fn ort_asset() -> Result<Vec<u8>, String> {
    let path = ort_path();
    let bytes = std::fs::read(&path).map_err(|error| {
        format!(
            "ORT 运行时不可读：{}（{error}）。服务端在 [echomuse].enabled=true 启动时会自动\
             从官方钉版 AAR 解出（OWW_ORT_URL 可指内网镜像）",
            path.display()
        )
    })?;
    if bytes.is_empty() {
        return Err(format!("ORT 运行时是空文件：{}", path.display()));
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "oww_tests.rs"]
mod tests;
