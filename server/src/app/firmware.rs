//! 固件 OTA 托管（**通用多目标**）：`idf.py build` 产物（ESP32 app .bin）经 HTTP 上传到
//! 服务端持久卷，设备经 `/api/ota` 响应的 `firmware.{version,url}` 感知新版本并流式拉取自升级。
//!
//! ## 为什么不是"只保留一个最新版"
//! 同一个服务端要服务**不同型号 × 不同功能**的设备（esp32c3 / esp32s3，xiaozhi / 其它应用）。
//! 它们的内核架构与板级配置不同，固件二进制**不可互换**：把 s3 的包发给 c3 会直接变砖。
//! 因此托管按 **`功能(project) + 型号(model)`** 二维寻址，另可选 `板型(board)` 做更细粒度覆盖：
//!
//! ```text
//! <dir>/<project>/<model>/<board|_>/firmware.bin + meta.json
//! ```
//!
//! 匹配规则（[`find`]）：功能与型号必须**完全相同**；同型号下优先"板型精确匹配"的条目，
//! 否则回落到 `board` 为空的"该型号通用"条目。**无匹配 = 不下发升级地址**（设备保持原样）。
//!
//! ## 元数据自动识别（型号/版本无需人工填）
//! ESP-IDF app 镜像自带元数据：镜像头偏移 12 是 `chip_id`（u16 LE），其后
//! `esp_image_header_t(24) + esp_image_segment_header_t(8)` 处是 `esp_app_desc_t(256)`，
//! 魔数 `0xABCD5432`，内含 `version` / `project_name` / `idf_ver` / 编译日期时间。
//! 上传时解析这些字段 → 型号与版本**自动填好**；解析失败（非 ESP-IDF 镜像）才要求查询参数兜底。
//! `chip_id → 型号名` 与固件 `SystemInfo::GetChipModelName()`（= `CONFIG_IDF_TARGET`）同表，
//! 因此与设备 OTA 请求里的 `chip_model_name` 可直接字符串比较。
//!
//! ## 上传（管理动作，expected_token 非空时需 Bearer 鉴权）
//! ```bash
//! curl -H "Authorization: Bearer <expected_token>" \
//!      --data-binary @build/xiaozhi-esp32.bin \
//!      "http://<host>:8000/api/ota/firmware"
//! # 包内元数据缺失/非 ESP-IDF 镜像时显式覆盖：
//! #   ?project=xiaozhi&model=esp32s3&version=2.0.4[&board=bread-compact-wifi]
//! ```
//!
//! ## 下载（设备动作）
//! `GET /api/ota/firmware/latest?project=&model=&board=` → 裸 app bin 流。设备侧
//! `Ota::Upgrade` 依赖 **200 + Content-Length 非零**（缺失即判定失败）。
//!
//! ## 兼容旧版
//! 旧版把单个固件放在根目录（`firmware.bin` + `latest.json`）。它没有型号信息，继续下发
//! 等于把不确定的包发给任何设备，故扫描时仅作为 [`LEGACY`] 条目**列出/可删除**，不参与匹配。

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 单个固件上传上限（ESP32 app 一般 1.5~4MB，留足余量）。
pub const MAX_FIRMWARE_BYTES: usize = 64 * 1024 * 1024;
/// 固件二进制固定文件名（同一 key 上传即覆盖，不留历史版本）。
const BIN_FILENAME: &str = "firmware.bin";
const META_FILENAME: &str = "meta.json";
/// `board` 缺省时的目录名：表示"该功能+型号通用"（不绑定具体板型）。
const BOARD_ANY: &str = "_";
/// 旧版单固件（根目录 `firmware.bin`/`latest.json`）的保留标识：只列出、可删除，不参与匹配。
const LEGACY: &str = "_legacy";

/// 托管固件元数据（`meta.json`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareMeta {
    /// 功能名（固件 `esp_app_desc_t.project_name`，如 `xiaozhi`）↔ 设备 `application.name`。
    pub project: String,
    /// 型号（镜像头 `chip_id` → `esp32c3`/`esp32s3`…）↔ 设备 `chip_model_name`。
    pub model: String,
    /// 可选板型（设备 `board.type`，如 `bread-compact-wifi`）；空 = 该功能+型号通用。
    #[serde(default)]
    pub board: String,
    /// 版本（`esp_app_desc_t.version`，数字点分段）。
    pub version: String,
    pub size: u64,
    pub sha256: String,
    /// 编译该固件的 IDF 版本（`esp_app_desc_t.idf_ver`）。
    #[serde(default)]
    pub idf_ver: String,
    /// 固件编译时刻（`date time`，来自 app 描述符）。
    #[serde(default)]
    pub built_at: String,
    /// 上传时刻（Unix 秒）。
    pub uploaded_at: u64,
}

impl FirmwareMeta {
    /// `GET /api/ota/firmware` 的一行（客户端只做展示，字段名即契约）。
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "project": self.project,
            "model": self.model,
            "board": self.board,
            "version": self.version,
            "size": self.size,
            "sha256": self.sha256,
            "idf_ver": self.idf_ver,
            "built_at": self.built_at,
            "uploaded_at": self.uploaded_at,
            "legacy": self.project == LEGACY,
        })
    }
}

/// 从 app 镜像里解析出的元数据（上传时用于自动填写型号/版本）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppImageInfo {
    /// `esp_app_desc_t.project_name`（功能标识）。
    pub project: String,
    /// 镜像头 `chip_id` 映射出的型号名（未知芯片 → 空串）。
    pub model: String,
    /// `esp_app_desc_t.version`。
    pub version: String,
    pub idf_ver: String,
    /// `date time`（两者都空 → 空串）。
    pub built_at: String,
}

// ============================================================
// ESP-IDF app 镜像解析
// ============================================================

/// `esp_app_desc_t.magic_word`（esp_app_format.h）。
const APP_DESC_MAGIC: u32 = 0xABCD_5432;
/// 镜像头 `esp_image_header_t` 长度。
const IMAGE_HEADER_LEN: usize = 24;
/// 首个段头 `esp_image_segment_header_t`（load_addr + data_len）。
const SEGMENT_HEADER_LEN: usize = 8;
/// `esp_app_desc_t` 在镜像中的偏移（段数据起点 = 头部之后）。
const APP_DESC_OFFSET: usize = IMAGE_HEADER_LEN + SEGMENT_HEADER_LEN;
/// `esp_app_desc_t` 长度（含保留字段）。
const APP_DESC_LEN: usize = 256;
/// 镜像头里 `chip_id` 的偏移（u16 LE）。
const CHIP_ID_OFFSET: usize = 12;

/// `esp_chip_id_t` → 型号名。取值与固件 `CONFIG_IDF_TARGET`（`SystemInfo::GetChipModelName()`）
/// 一致，故可与设备 OTA 请求的 `chip_model_name` 直接比较；未知 id → None。
pub fn chip_model_name(chip_id: u16) -> Option<&'static str> {
    Some(match chip_id {
        0x0000 => "esp32",
        0x0002 => "esp32s2",
        0x0005 => "esp32c3",
        0x0009 => "esp32s3",
        0x000C => "esp32c2",
        0x000D => "esp32c6",
        0x0010 => "esp32h2",
        0x0012 => "esp32p4",
        0x0014 => "esp32c61",
        0x0017 => "esp32c5",
        _ => return None,
    })
}

/// 定长 C 字符串字段 → String（到 NUL 或字段尾为止，去首尾空白）。
fn read_cstr(bytes: &[u8], offset: usize, len: usize) -> String {
    let end = bytes[offset..offset + len]
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(len);
    String::from_utf8_lossy(&bytes[offset..offset + end])
        .trim()
        .to_string()
}

/// 解析 ESP-IDF app 镜像的元数据；非 ESP-IDF app 镜像（魔数不符/长度不足）→ None。
pub fn parse_app_image(bytes: &[u8]) -> Option<AppImageInfo> {
    if bytes.len() < APP_DESC_OFFSET + APP_DESC_LEN {
        return None;
    }
    let magic = u32::from_le_bytes(
        bytes[APP_DESC_OFFSET..APP_DESC_OFFSET + 4]
            .try_into()
            .ok()?,
    );
    if magic != APP_DESC_MAGIC {
        return None;
    }
    let chip_id = u16::from_le_bytes(
        bytes[CHIP_ID_OFFSET..CHIP_ID_OFFSET + 2]
            .try_into()
            .ok()?,
    );
    let version = read_cstr(bytes, APP_DESC_OFFSET + 16, 32);
    let project = read_cstr(bytes, APP_DESC_OFFSET + 48, 32);
    let time = read_cstr(bytes, APP_DESC_OFFSET + 80, 16);
    let date = read_cstr(bytes, APP_DESC_OFFSET + 96, 16);
    let idf_ver = read_cstr(bytes, APP_DESC_OFFSET + 112, 32);
    Some(AppImageInfo {
        project,
        model: chip_model_name(chip_id).unwrap_or("").to_string(),
        version,
        idf_ver,
        built_at: if date.is_empty() {
            time
        } else {
            format!("{date} {time}")
        },
    })
}

// ============================================================
// 存储布局
// ============================================================

/// 固件存储根目录：`XIAOZHI_FIRMWARE_DIR`（默认 /data/firmware，随 compose 卷持久化）。
pub fn firmware_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("XIAOZHI_FIRMWARE_DIR").unwrap_or_else(|_| "/data/firmware".to_string()),
    )
}

/// 路径段白名单：字母数字 + `.`/`-`/`_`，长度 ≤ 64，且**至少含一个字母数字**。
///
/// 型号/功能/板型会拼进存储目录，版本会拼进 URL；不可放行 `/`、`\`、`?` 等分隔符，
/// 也要挡住 `.` / `..` 这类纯点段（只允许点号会让它们穿透目录）。
fn sanitize_segment(raw: &str) -> Option<String> {
    let v = raw.trim();
    if v.is_empty() || v.len() > 64 {
        return None;
    }
    if !v
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return None;
    }
    if !v.chars().any(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(v.to_string())
}

/// 条目目录（调用方传入**已净化**的段）。
fn entry_dir(root: &Path, project: &str, model: &str, board: &str) -> PathBuf {
    root.join(project)
        .join(model)
        .join(if board.is_empty() { BOARD_ANY } else { board })
}

/// 条目二进制路径（旧版条目落在根目录）。
fn bin_path_in(root: &Path, meta: &FirmwareMeta) -> PathBuf {
    if meta.project == LEGACY && meta.model == LEGACY {
        return root.join(BIN_FILENAME);
    }
    entry_dir(root, &meta.project, &meta.model, &meta.board).join(BIN_FILENAME)
}

/// 临时文件 + rename 落盘（断电不留半包）；rename 失败（个别挂载）回退原地写。
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        std::fs::write(path, bytes)?;
    }
    Ok(())
}

/// 目录下的子目录（排序，保证扫描结果稳定）。
fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

fn read_meta(path: &Path) -> Option<FirmwareMeta> {
    let data = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<FirmwareMeta>(&data) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!("固件元数据解析失败（忽略 {}）: {e}", path.display());
            None
        }
    }
}

/// 旧版根目录元数据（只有 version/size/sha256/uploaded_at）。
#[derive(Debug, Deserialize)]
struct LegacyMeta {
    version: String,
    size: u64,
    sha256: String,
    uploaded_at: u64,
}

/// 旧版单固件 → [`LEGACY`] 条目（不参与设备匹配，仅供管理页列出与删除）。
fn legacy_meta(root: &Path) -> Option<FirmwareMeta> {
    let data = std::fs::read_to_string(root.join("latest.json")).ok()?;
    let old: LegacyMeta = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("旧版 latest.json 解析失败（忽略）: {e}");
            return None;
        }
    };
    Some(FirmwareMeta {
        project: LEGACY.to_string(),
        model: LEGACY.to_string(),
        board: String::new(),
        version: old.version,
        size: old.size,
        sha256: old.sha256,
        idf_ver: String::new(),
        built_at: String::new(),
        uploaded_at: old.uploaded_at,
    })
}

/// 全部托管条目（新上传的在前）。每次调用扫盘：条目是个位数，无需额外索引文件
/// （索引一旦损坏就要重建，而扫盘永远与磁盘一致）。
pub fn scan() -> Vec<FirmwareMeta> {
    scan_in(&firmware_dir())
}

fn scan_in(root: &Path) -> Vec<FirmwareMeta> {
    let mut out = Vec::new();
    if let Some(m) = legacy_meta(root) {
        out.push(m);
    }
    for project in read_dirs(root) {
        for model in read_dirs(&project) {
            for board in read_dirs(&model) {
                if let Some(m) = read_meta(&board.join(META_FILENAME)) {
                    out.push(m);
                }
            }
        }
    }
    out.sort_by(|a, b| b.uploaded_at.cmp(&a.uploaded_at));
    out
}

/// 按 **功能 + 型号** 找托管固件（同型号下优先板型精确匹配，其次该型号通用条目）。
///
/// 功能/型号任一为空 → None：**没有型号信息绝不下发**，否则就是"把不确定的包发给设备"。
pub fn find(project: &str, model: &str, board: &str) -> Option<FirmwareMeta> {
    find_in(&firmware_dir(), project, model, board)
}

fn find_in(root: &Path, project: &str, model: &str, board: &str) -> Option<FirmwareMeta> {
    if project.is_empty() || model.is_empty() {
        return None;
    }
    let mut generic: Option<FirmwareMeta> = None;
    for m in scan_in(root) {
        // 旧版条目没有型号信息：永不参与匹配（即便有人拿 _legacy 当标识来查）
        if m.project == LEGACY || m.project != project || m.model != model {
            continue;
        }
        if !board.is_empty() && m.board == board {
            return Some(m);
        }
        if m.board.is_empty() && generic.is_none() {
            generic = Some(m);
        }
    }
    generic
}

/// 删除一个条目（旧版条目删根目录两文件）；不存在 → false。顺手清理空掉的父目录。
pub fn remove(project: &str, model: &str, board: &str) -> bool {
    remove_in(&firmware_dir(), project, model, board)
}

fn remove_in(root: &Path, project: &str, model: &str, board: &str) -> bool {
    if project == LEGACY && model == LEGACY {
        let a = std::fs::remove_file(root.join(BIN_FILENAME)).is_ok();
        let b = std::fs::remove_file(root.join("latest.json")).is_ok();
        return a || b;
    }
    let dir = entry_dir(root, project, model, board);
    if !dir.is_dir() {
        return false;
    }
    let _ = std::fs::remove_dir_all(&dir);
    let mut parent = dir.parent().map(Path::to_path_buf);
    while let Some(p) = parent {
        if p == *root || std::fs::remove_dir(&p).is_err() {
            break; // 到根 / 非空即停
        }
        parent = p.parent().map(Path::to_path_buf);
    }
    true
}

// ============================================================
// 版本比较
// ============================================================

/// 数字点分段版本（`2.0.4` → `[2,0,4]`）；任一段非纯数字 → None。
fn parse_version(v: &str) -> Option<Vec<u64>> {
    let mut out = Vec::new();
    for seg in v.trim().split('.') {
        if seg.is_empty() || !seg.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        out.push(seg.parse().ok()?);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 托管版本是否比设备当前版本更新。语义对齐固件 `Ota::IsNewVersionAvailable`：
/// 逐段比较，前缀相同则段数多者更新。任一侧不可解析 → `false`——宁可不下发，
/// 也不能让畸形版本号触发降级或无限升级循环。
pub fn is_newer(hosted: &str, current: &str) -> bool {
    let (Some(h), Some(c)) = (parse_version(hosted), parse_version(current)) else {
        return false;
    };
    for i in 0..h.len().min(c.len()) {
        match h[i].cmp(&c[i]) {
            Ordering::Greater => return true,
            Ordering::Less => return false,
            Ordering::Equal => {}
        }
    }
    h.len() > c.len()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ============================================================
// HTTP 层
// ============================================================
//
// 路由与四个处理器在子模块（`firmware/http.rs`）：存储/匹配（本模块）要能被单测直接调，
// HTTP 契约（状态码/鉴权/体积上限）是另一件事；拆开也满足 §5.10 的 600 行约定。
pub mod http;
pub use http::router;

#[cfg(test)]
mod tests;
