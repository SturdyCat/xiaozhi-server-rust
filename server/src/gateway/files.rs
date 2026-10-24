//! 服务端数据目录的**通用文件管理**（管理台"文件"页数据源，`/api/files/*`）：
//! 列表 / 建目录 / 上传 / 下载 / 删除。管理台与脚本经同一组 HTTP 端点维护服务器
//! 持久卷上的文件；EchoMuse 自定义唤醒词模型（[`OWW_MODELS_DIR`]）由此获得上传通道。
//!
//! ## 存储根（单一来源）
//! 根 = [`crate::device::echomuse::registry::data_dir`]（`[memory].db_path` 的父目录，
//! 默认 `/data`）——与配置库、设备登记文件同卷，不新增配置项，也不复制回退逻辑。
//!
//! ## 安全边界（纯函数见 [`validate_rel`] / [`resolve_in`] / [`is_protected`]）
//! - 相对路径校验：拒绝 `..`/`.` 段、空段（`//`、前导 `/`）、`\` 与 NUL；尾部 `/` 仅
//!   视作目录习惯写法去掉。`""` = 根本身（仅列表允许）。
//! - 解析后对**最近的存在祖先** `canonicalize` 并校验 `starts_with(root)`，符号链接
//!   外逃一律拒绝；目标尚不存在（upload/mkdir）时把剩余段拼回已验证前缀。
//! - 受保护名 [`PROTECTED_NAMES`]：SQLite 配置+记忆库（`data.db`/`-wal`/`-shm`）。
//!   任意层级最后一段命中即禁删、禁上传覆盖/新建（403，文案写明 配置/记忆库文件）。
//! - 根目录本身不可删、不可作为上传目标（400）；删除目录即递归删除（文件夹删除入口）。
//! - 上传经 [`write_atomic`]（临时文件 + rename，断电不留半包），上限与固件托管一致
//!   （[`MAX_FILE_BYTES`]）。
//!
//! ## 鉴权与门控
//! 全部端点走 [`crate::gateway::ws::auth_ok`]（`expected_token` 空 = 不校验；头或 `?token=`
//! 均可，web fetch/浏览器拿不到自定义头）。**EchoMuse enabled 开关不作用于此模块**——
//! 这是通用服务端存储管理，不是设备面。错误一律 JSON `{"error":"…"}` + 相应状态码。
//!
//! ## 客户端兼容
//! Kuikly NetworkModule 只有 GET/POST：删除提供 `POST /api/files/delete`（body 带
//! path），`DELETE /api/files?path=` 供 curl/脚本；两者同一处理器。上传 body 为裸
//! octet-stream 字节（与固件上传同款）。端点契约细节见 [`http`] 与 `files/AGENTS.md`。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{http::StatusCode, Router};

use crate::config::Config;
use crate::runtime::Engines;

/// EchoMuse 自定义唤醒词模型目录（相对存储根）：管理台文件页 / API 把训练好的
/// `.onnx` 传到这里，设备设置页的模型下拉据 `GET /api/files?path=oww-models` 列出。
/// 服务端不主动建目录（upload/mkdir 按需创建）；且**不做** 服务端→设备的模型分发，
/// 设备只消费本地已装的模型（见 `docs/echomuse.md` 自定义唤醒词一节）。
/// 缺失时的列表语义见 [`OPTIONAL_DIRS`]。
pub const OWW_MODELS_DIR: &str = "oww-models";

/// 「允许缺失」的业务目录（相对存储根）：**不存在时 `GET /api/files` 返回空清单 200**，
/// 而不是 404。
///
/// 为什么需要这个白名单：管理台一打开（`AdminPage.refreshAll`）就会无条件查一次
/// `oww-models/` 来填唤醒词下拉，而该目录在没人上传过模型前本就不存在。按 404 返回会
/// 让**每次启动都刷一条客户端错误日志**（Kuikly `NetworkModule` 对任何非 2xx 一律
/// `[kuikly error]Received a non-success status code: 404`，业务侧静默也压不住），
/// 看起来像故障、实际是正常初始态。
///
/// 边界：只对**顶层**目录名精确命中生效，子路径（`oww-models/x`）与其它未知路径仍走
/// 404；上传/建目录依然按需创建，不预创建任何目录（保持「存储层不造业务副作用」）。
const OPTIONAL_DIRS: [&str; 1] = [OWW_MODELS_DIR];

/// 单文件上传上限：与固件托管同值（模型 ~1MB，音频/文档留足余量）。
pub const MAX_FILE_BYTES: usize = crate::device::firmware::MAX_FIRMWARE_BYTES;

/// 受保护文件名（SQLite 配置+记忆库）：最后一段命中即受保护，任意层级同规则。
const PROTECTED_NAMES: [&str; 3] = ["data.db", "data.db-wal", "data.db-shm"];

// ============================================================
// Context：处理器与存储根解耦（可测性，模式同 echomuse::ble_http）
// ============================================================

/// 处理器只依赖配置（鉴权 + 存储根推导），不触真实引擎：
/// 生产用 [`Arc<Engines>`]，测试用自带 `Config` 的桩。
pub(super) trait Context: Clone + Send + Sync + 'static {
    fn config(&self) -> &Config;
}

impl Context for Arc<Engines> {
    fn config(&self) -> &Config {
        &self.config
    }
}

/// 文件管理存储根：复用 echomuse registry 的数据目录推导（单一来源）。
fn storage_root(cfg: &Config) -> PathBuf {
    crate::device::echomuse::registry::data_dir(cfg)
}

// ============================================================
// 纯逻辑：路径校验 / 受保护名 / 解析
// ============================================================

/// 校验并规整相对路径（相对存储根）。
///
/// 返回 `Ok("")` = 根本身（仅列表语义允许，删除/上传在上层拒绝）；
/// `Err` 为用户可读原因（处理器原样进 `{"error"}`）。
pub(super) fn validate_rel(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/'); // 尾部 `/` 是目录习惯写法
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.contains('\\') {
        return Err("路径不能包含反斜杠".into());
    }
    if trimmed.contains('\0') {
        return Err("路径不能包含 NUL 字节".into());
    }
    let mut segs = Vec::new();
    for seg in trimmed.split('/') {
        match seg {
            "" => return Err("路径含空段（不允许 // 或以 / 开头）".into()),
            "." => return Err("路径不允许 `.` 段".into()),
            ".." => return Err("路径不允许 `..` 上跳".into()),
            s => segs.push(s),
        }
    }
    Ok(segs.join("/"))
}

/// 是否受保护名：rel 的**最后一段**命中 [`PROTECTED_NAMES`]（任意层级同规则）。
pub(super) fn is_protected(rel: &str) -> bool {
    rel.rsplit('/')
        .next()
        .is_some_and(|n| PROTECTED_NAMES.contains(&n))
}

/// 403 文案：必须说清为什么（删/覆盖了配置+记忆库文件服务端就瘫）。
pub(super) fn protected_reason() -> &'static str {
    "受保护文件：SQLite 配置/记忆库（data.db、data.db-wal、data.db-shm），不允许删除或覆盖"
}

/// 把已校验的 rel 解析为根内的真实路径；`None` = 根不存在或符号链接外逃。
///
/// 目标已存在 → 直接 canonicalize 并校验在根内；目标不存在（upload/mkdir）→
/// 对最近的存在祖先 canonicalize 校验后拼回剩余段（未存在的段不可能含符号链接）。
pub(super) fn resolve_in(root: &Path, rel: &str) -> Option<PathBuf> {
    let root_c = std::fs::canonicalize(root).ok()?;
    if rel.is_empty() {
        return Some(root_c);
    }
    let target = root.join(rel);
    if let Ok(t) = std::fs::canonicalize(&target) {
        return t.starts_with(&root_c).then_some(t);
    }
    let mut anc = target.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match std::fs::canonicalize(&anc) {
            Ok(t) => {
                if !t.starts_with(&root_c) {
                    return None;
                }
                let mut resolved = t;
                for seg in tail.iter().rev() {
                    resolved.push(seg);
                }
                return Some(resolved);
            }
            Err(_) => {
                tail.push(anc.file_name()?.to_os_string());
                anc.pop();
            }
        }
    }
}

// ============================================================
// 纯逻辑：目录列表
// ============================================================

/// 目录列表一行（`GET /api/files` 的 entries 元素，字段名即契约）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    pub name: String,
    pub dir: bool,
    /// 字节数（目录恒 0）。
    pub size: u64,
    /// 最后修改时间（Unix 秒；取不到为 0）。
    pub modified: u64,
    /// 最后一段是否受保护名（403 入口提示，管理页可据此禁用按钮）。
    pub protected: bool,
}

/// 列目录：目录在前、同名节按名称排序。符号链接条目按链接本身展示（`DirEntry::
/// metadata` 不跟随），列表只做展示，不做追踪。
pub(super) fn list_entries(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let meta = e.metadata()?;
        let name = e.file_name().to_string_lossy().into_owned();
        out.push(Entry {
            dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0),
            protected: PROTECTED_NAMES.contains(&name.as_str()),
            name,
        });
    }
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

// ============================================================
// 操作层：契约状态码在这里定，HTTP 层只做提取/鉴权/JSON 形状
// ============================================================

/// 操作失败（HTTP 契约状态码 + 用户可读文案），处理器原样转 `{"error"}`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OpError {
    pub status: StatusCode,
    pub message: String,
}

impl OpError {
    pub(super) fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

fn bad(message: impl Into<String>) -> OpError {
    OpError::new(StatusCode::BAD_REQUEST, message)
}

fn not_found(what: &str, rel: &str) -> OpError {
    OpError::new(StatusCode::NOT_FOUND, format!("{what}不存在: {rel}"))
}

fn internal(what: &str, e: std::io::Error) -> OpError {
    OpError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{what}: {e}"))
}

/// 列表：返回 canonical 根（响应的 `root` 字段）+ 排序条目。
///
/// 一般路径缺失 404、存在但不是目录 400；但 [`OPTIONAL_DIRS`] 里的业务目录缺失时按
/// **空目录 200** 返回（见常量说明）。
pub(super) fn op_list(root: &Path, rel: &str) -> Result<(PathBuf, Vec<Entry>), OpError> {
    // 根尚不存在（首次部署、卷未挂载）与「目录不存在」同一语义：404。
    let root_c = std::fs::canonicalize(root).map_err(|_| not_found("目录", rel))?;
    let dir = resolve_in(root, rel).ok_or_else(|| not_found("目录", rel))?;
    if dir.is_dir() {
        let entries = list_entries(&dir).map_err(|e| internal("读取目录失败", e))?;
        return Ok((root_c, entries));
    }
    if dir.exists() {
        return Err(bad(format!("路径不是目录: {rel}")));
    }
    if OPTIONAL_DIRS.contains(&rel) {
        return Ok((root_c, Vec::new()));
    }
    Err(not_found("目录", rel))
}

/// 建目录：父目录一并创建；目录已存在 = 幂等成功；同名文件已存在 = 409。
pub(super) fn op_mkdir(root: &Path, rel: &str) -> Result<(), OpError> {
    if rel.is_empty() {
        return Err(bad("缺少 path（不能对根目录建目录）"));
    }
    let _ = std::fs::create_dir_all(root); // 根缺失时先补齐（与 upload 同）
    let target =
        resolve_in(root, rel).ok_or_else(|| bad(format!("路径解析失败（符号链接外逃）: {rel}")))?;
    if target.is_dir() {
        return Ok(());
    }
    if target.exists() {
        return Err(OpError::new(
            StatusCode::CONFLICT,
            format!("同名文件已存在，不能建目录: {rel}"),
        ));
    }
    std::fs::create_dir_all(&target).map_err(|e| internal("创建目录失败", e))
}

/// 上传（裸字节）：自动建父目录；已存在目标未带 overwrite = 409；目标为目录 = 409；
/// 最后一段受保护 = 403。返回写入字节数。
pub(super) fn op_upload(
    root: &Path,
    rel: &str,
    bytes: &[u8],
    overwrite: bool,
) -> Result<u64, OpError> {
    if rel.is_empty() {
        return Err(bad("上传目标不能是根目录"));
    }
    if is_protected(rel) {
        return Err(OpError::new(StatusCode::FORBIDDEN, protected_reason()));
    }
    let _ = std::fs::create_dir_all(root);
    let target =
        resolve_in(root, rel).ok_or_else(|| bad(format!("路径解析失败（符号链接外逃）: {rel}")))?;
    let parent = target
        .parent()
        .ok_or_else(|| bad(format!("路径没有父目录: {rel}")))?;
    std::fs::create_dir_all(parent).map_err(|e| internal("创建父目录失败", e))?;
    if target.is_dir() {
        return Err(OpError::new(
            StatusCode::CONFLICT,
            format!("目标路径是已存在的目录: {rel}"),
        ));
    }
    if target.exists() && !overwrite {
        return Err(OpError::new(
            StatusCode::CONFLICT,
            format!("目标已存在（覆盖请带 ?overwrite=1）: {rel}"),
        ));
    }
    write_atomic(&target, bytes).map_err(|e| internal("写入文件失败", e))?;
    Ok(bytes.len() as u64)
}

/// 删除：文件或目录（递归）。缺失 = 404；最后一段受保护 = 403；根 = 400。
pub(super) fn op_delete(root: &Path, rel: &str) -> Result<(), OpError> {
    if rel.is_empty() {
        return Err(bad("拒绝删除根目录本身"));
    }
    if is_protected(rel) {
        return Err(OpError::new(StatusCode::FORBIDDEN, protected_reason()));
    }
    let target = resolve_in(root, rel).ok_or_else(|| not_found("文件或目录", rel))?;
    if !target.exists() {
        return Err(not_found("文件或目录", rel));
    }
    let done = if target.is_dir() {
        std::fs::remove_dir_all(&target)
    } else {
        std::fs::remove_file(&target)
    };
    done.map_err(|e| internal("删除失败", e))
}

/// 下载：返回（文件名，字节）。缺失 = 404；目录 = 400。
pub(super) fn op_download(root: &Path, rel: &str) -> Result<(String, Vec<u8>), OpError> {
    let target = resolve_in(root, rel).ok_or_else(|| not_found("文件", rel))?;
    if !target.exists() {
        return Err(not_found("文件", rel));
    }
    if target.is_dir() {
        return Err(bad(format!(
            "路径是目录，不能下载（先打包或逐文件取）: {rel}"
        )));
    }
    let bytes = std::fs::read(&target).map_err(|e| internal("读取文件失败", e))?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    Ok((name, bytes))
}

/// 临时文件 + rename 落盘（断电不留半包）；rename 失败（个别挂载）回退原地写。
/// 与 `firmware::write_atomic` 同款模式（本地副本：通用文件模块不反向依赖固件实现）。
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        std::fs::write(path, bytes)?;
    }
    Ok(())
}

// ============================================================
// HTTP 层
// ============================================================
//
// 路由与处理器在子模块（`files/http.rs`）：路径校验/受保护判定/操作契约（本模块）
// 要能被单测直接调；状态码与 JSON 形状以外的提取（query/body/鉴权头）是另一件事。
pub mod http;

/// 文件管理子路由（并入主 router；上传体积上限仅作用于本组）。
pub fn router() -> Router<Arc<Engines>> {
    http::router::<Arc<Engines>>()
}

#[cfg(test)]
mod tests;

/// 真实 loopback HTTP 回归（TestContext 起 axum，不起真实 Engines）。
#[cfg(test)]
mod http_tests;
