//! `/api/config` 读写端点组（含配置来源元信息）：
//! - `GET`：启动指定了配置文件则实时读盘，否则返回内存内置默认配置；**密钥打码**
//!   ——只回 `has_<field>` presence 标记，不回明文（见 [`SECRET_PATHS`]）。
//! - `PUT/POST`：**部分更新**语义（未出现的字段保持、显式 `null` 清除、密钥空串 = 保留原值）；
//!   写回启动加载的配置文件（TOML，原注释会覆盖丢失）；内置默认启动时 400。
//!   响应带 `hot` / `hot_hint` / `changed_sections`：本次保存**实际**的生效档位
//!   （见 [`impact`]，取自注册表字段元数据里最严格的一项）。
//! - `GET /api/config/meta`：配置来源（路径 / 是否挂载卷持久化），排查"配置丢失去向"。
//!
//! 拆分子模块（原 `ws.rs`）：路由表与 WS 网关见 [`super`]。

use std::sync::Arc;

use anyhow::Context;
use axum::{
    extract::{Json, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::config::Config;
use crate::engine::Engines;

mod impact;
use impact::{save_impact, SaveImpact};

/// 当前生效配置：启动时指定了配置文件则实时读盘，否则为内存内置默认配置。
///
/// 健壮性：磁盘配置损坏/旧 schema 解析失败时回退内存配置（200），让管理页能正常打开
/// ——一次成功保存即用合法 TOML 覆盖修复坏文件，避免陷入"打不开 → 永远修不好"的死局。
/// 原因记入服务端日志。
///
/// `GET`（填表单）与 `PUT`（部分更新的基线）**共用**本函数：两者必须看到同一份"现值"，
/// 否则会出现"表单显示 A、保存却按 B 合并"的错位。
pub(crate) fn current_config(engines: &Engines) -> Config {
    match &engines.config_path {
        Some(p) => match Config::load(p) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(
                    "读取配置文件 {p} 失败，回退内存配置（可在管理页重新保存修复）: {e}"
                );
                (*engines.config).clone()
            }
        },
        None => (*engines.config).clone(),
    }
}

/// 剔除 GET 响应里的**包装字段**（`revision` / `expected_revision`）：
/// 客户端把整份 GET 结果原样回传时，它们不属于 `Config`，不该混进持久化结构。
fn strip_wrapper_keys(v: &mut serde_json::Value) {
    if let Some(obj) = v.as_object_mut() {
        obj.remove("revision");
        obj.remove("expected_revision");
    }
}

/// 敏感字段路径表：GET 打码、POST「空串 = 保留原值」、客户端 presence 三者共用的**唯一权威**。
///
/// ⚠️ 三者必须**同批**上线。只打码不改 POST 语义 → 旧表单/旧客户端把空串回传，保存一次
/// 就把密钥清空（`merge_patch` 是"出现即覆盖"）；只改语义不打码 → 密钥继续明文回传。
const SECRET_PATHS: &[&[&str]] = &[
    &["llm", "api_key"],
    &["tts", "xfyun", "api_key"],
    &["tts", "xfyun", "api_secret"],
    &["aiui", "api_key"],
    &["aiui", "api_secret"],
    // 记忆抽取的独立模型路由（空 = 复用 [llm]）
    &["memory", "extractor", "api_key"],
];

/// presence 标记字段名：`api_key` → `has_api_key`（`api_secret` → `has_api_secret`）。
fn presence_key(leaf: &str) -> String {
    format!("has_{leaf}")
}

/// 在 JSON 中定位路径的**父对象**与叶子键；路径中缺任一层即返回 `None`（**不创建**中间对象）。
fn secret_parent<'a>(
    v: &'a mut serde_json::Value,
    path: &'a [&'a str],
) -> Option<(&'a mut serde_json::Map<String, serde_json::Value>, &'a str)> {
    let (leaf, parents) = path.split_last()?;
    let mut cur = v;
    for p in parents {
        cur = cur.as_object_mut()?.get_mut(*p)?;
    }
    Some((cur.as_object_mut()?, leaf))
}

/// GET 打码：密钥字段**一律从响应中摘除**，只留 `has_<leaf>` 布尔标记（空值记 `false`）。
///
/// 为什么是"摘掉"而不是替换成 `"***"`：客户端会把 GET 结果当基线回传，
/// `"***"` 会被 `merge_patch` 当作真密钥覆盖写回。摘掉后"未发送"= 保留原值，语义闭合。
/// 空值也一并摘除：响应里**永远不出现**密钥字段，客户端只能依赖 presence 标记。
fn redact_secrets(v: &mut serde_json::Value) {
    for path in SECRET_PATHS {
        let Some((obj, leaf)) = secret_parent(v, path) else {
            continue;
        };
        let present = obj
            .get(leaf)
            .and_then(|x| x.as_str())
            .is_some_and(|s| !s.trim().is_empty());
        obj.remove(leaf);
        obj.insert(presence_key(leaf), serde_json::Value::Bool(present));
    }
}

/// 旧路径重写（P4 配置约定统一，见 `crate::config` 模块文档）：
/// 把 P4 之前的写法折到规范路径，使**旧客户端 / 旧表单**的补丁仍然生效。
///
/// 为什么必须做：规范位置按"新位置优先"合并（`Config::normalize` 只在子段仍是默认值时才搬旧字段），
/// 若客户端补丁写的是旧位置、而盘上规范位置已有非默认值，那份改动会被**静默丢弃**——
/// 静默丢弃正是本仓库最忌讳的失败模式（配置改了却像没改）。
///
/// 覆盖：`tts.backend → tts.engine`、`tts.<旧扁平字段> → tts.kokoro.<字段>`、
/// `memory.backend → memory.engine`。**同一补丁里显式的新位置优先**（`or_insert` 不覆盖）。
fn rewrite_legacy_paths(patch: &mut serde_json::Value) {
    let Some(root) = patch.as_object_mut() else {
        return;
    };
    if let Some(tts) = root.get_mut("tts").and_then(|v| v.as_object_mut()) {
        if !tts.contains_key("engine") {
            if let Some(backend) = tts.remove("backend") {
                tts.insert("engine".into(), backend);
            }
        }
        let mut moved: Vec<(String, serde_json::Value)> = Vec::new();
        for key in crate::plugins::tts::LEGACY_BODY_FIELDS {
            if let Some(v) = tts.remove(*key) {
                moved.push(((*key).to_string(), v));
            }
        }
        if !moved.is_empty() {
            match tts
                .entry("kokoro")
                .or_insert_with(|| serde_json::json!({}))
            {
                serde_json::Value::Object(kokoro) => {
                    for (k, v) in moved {
                        kokoro.entry(k).or_insert(v); // 显式新位置优先
                    }
                }
                // `[tts.kokoro]` 写了非对象：保留原值，交给反序列化报精确类型错误
                other => tracing::warn!(
                    "PUT /api/config：[tts.kokoro] 不是对象（{}），旧扁平字段未迁移（将由解析报错）",
                    other
                ),
            }
        }
    }
    if let Some(mem) = root.get_mut("memory").and_then(|v| v.as_object_mut()) {
        if !mem.contains_key("engine") {
            if let Some(backend) = mem.remove("backend") {
                mem.insert("engine".into(), backend);
            }
        }
    }
}

/// POST 前置处理（在 [`merge_patch`] **之前**跑）：
/// - **旧路径重写**（P4）：`tts.backend`/`tts.model`/`memory.backend` → 规范位置；
/// - **空串密钥 → 从补丁里删掉** = "未修改，保留原值"（旧客户端会原样回传空串）；
/// - presence 标记与 `revision` 同为**响应包装字段**，不属于 `Config`，一并剔除；
/// - 显式 `null` **保留**（由 `merge_patch` 解释为"清除"），非空值照常覆盖。
fn sanitize_patch(patch: &mut serde_json::Value) {
    rewrite_legacy_paths(patch);
    for path in SECRET_PATHS {
        let Some((obj, leaf)) = secret_parent(patch, path) else {
            continue;
        };
        if obj
            .get(leaf)
            .and_then(|x| x.as_str())
            .is_some_and(|s| s.trim().is_empty())
        {
            obj.remove(leaf);
        }
        obj.remove(&presence_key(leaf));
    }
    strip_wrapper_keys(patch);
}

/// 现盘配置 + 客户端补丁 → 新配置：**`put_config` 与单测共用的唯一管线**。
///（此前测试里另有一份 `simulate_put` 复刻，容易与生产代码漂移——现已合并。）
pub(crate) fn merge_client_patch(
    base: &Config,
    patch: &serde_json::Value,
) -> serde_json::Result<Config> {
    let mut merged = serde_json::to_value(base)?;
    let mut patch = patch.clone();
    sanitize_patch(&mut patch);
    merge_patch(&mut merged, &patch);
    // 合并结果可能是"旧位置"形状（旧客户端补丁）：与读盘同一套规范化，
    // 保证**保存出去的文件**与**内存里跑的配置**都是规范形状。
    let mut cfg: Config = serde_json::from_value(merged)?;
    cfg.normalize();
    Ok(cfg)
}

/// 配置内容指纹（revision）：用于拒绝**过期写入**。
///
/// 场景：两个标签页/两个端（浏览器 + macApp 测试台）各打开一份表单，A 保存后 B 再保存，
/// 会把 A 的改动静默覆盖掉（last-write-wins）。有了指纹，B 的写入被拒并提示重新加载。
///
/// 用 FNV-1a 而非哈希库：无新依赖、跨进程稳定、够快（配置只有几 KB）。
fn config_revision(cfg: &Config) -> String {
    // 摘要基于**打码后**的表示：指纹不依赖密钥内容。
    // 好处有二：① 指纹不会成为密钥的离线比对依据；② 只改密钥不必让其他端全部 409
    //（密钥不会被空值覆盖，见 `sanitize_patch`，所以跳过其值不引入覆盖风险）。
    // presence 变化仍会改变指纹。
    let mut v = serde_json::to_value(cfg).unwrap_or(serde_json::Value::Null);
    redact_secrets(&mut v);
    let text = v.to_string();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// 读取当前配置（供管理页面 UI 填充表单）。
pub(super) async fn get_config(State(engines): State<Arc<Engines>>) -> Response {
    let cfg = current_config(&engines);
    // 附 `revision`（内容指纹）：客户端原样回传，用于拒绝过期写入。
    // 注意它是**响应包装字段**（不是 Config 的字段）；Config 无 deny_unknown_fields，
    // 即使客户端把它整份回传也不会解析失败（PUT 侧还会主动剔除）。
    let mut body = match serde_json::to_value(&cfg) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("序列化配置失败: {e}"),
            )
                .into_response()
        }
    };
    body["revision"] = serde_json::json!(config_revision(&cfg));
    // 密钥打码：明文字段摘除，只留 `has_<field>` presence 标记（客户端据此显示"已配置"）
    redact_secrets(&mut body);
    match serde_json::to_string(&body) {
        Ok(json) => (
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            json,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("序列化配置失败: {e}"),
        )
            .into_response(),
    }
}

/// 配置持久化（PUT /api/config 共用）：序列化为 TOML 写回。
///
/// P4 起先 `normalize()`（幂等）再序列化：手搓/未规范化的内存配置若直接落盘，
/// 会把 `[tts].engine` 写成空值、旧位置字段写回旧结构——那正是我们要消除的"保存即漂移"。
///
/// 写入策略两级：
/// 1. **原子写**（首选）：写同目录临时文件 → `rename` 覆盖，避免写一半被杀导致配置损坏；
/// 2. **原地写回退**：⚠️ compose 把 `config.toml` 以**单文件 bind mount** 挂进容器时，
///    `rename` 跨挂载点必然失败（`EBUSY`，os error 16，实测）——此时回退直接覆写原文件
///    （非原子，但挂载单文件的场景下这是唯一可行路径），并清理残留 tmp。
pub(crate) fn persist_config(path: &str, cfg: &Config) -> anyhow::Result<()> {
    let mut cfg = cfg.clone();
    cfg.normalize();
    let toml_str = toml::to_string_pretty(&cfg).context("序列化配置失败")?;
    let tmp = format!("{}.{}.tmp", path, uuid::Uuid::new_v4());
    let atomic = std::fs::write(&tmp, &toml_str).and_then(|()| std::fs::rename(&tmp, path));
    if atomic.is_ok() {
        return Ok(());
    }
    let atomic_err = atomic.unwrap_err();
    let fallback = std::fs::write(path, &toml_str);
    let _ = std::fs::remove_file(&tmp); // 清理 rename 失败残留的临时文件
    if let Err(e) = fallback {
        let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
            "（配置文件或所在目录为只读/属主不符。请检查宿主挂载是否误加 :ro，或修正属主与权限后重启 server。）"
        } else {
            ""
        };
        anyhow::bail!("写入配置失败: {e}{hint}（原子写失败原因: {atomic_err}）");
    }
    tracing::warn!(
        "配置已原地写入 {path}（挂载为单文件 bind mount，rename 原子替换不可用: {atomic_err}）"
    );
    Ok(())
}

/// 把管理页提交的 JSON **增量合并**到当前配置上（结构性修复，见
/// `docs/plugin-architecture-unification.md` §1.7 bug-1 与 §7 风险 2）。
///
/// 语义（对齐 DSH 的 *"Path edits preserve fields a client did not receive"*）：
/// - **出现的字段**覆盖现值；
/// - **未出现的字段保持不动** —— 旧客户端、或新增配置段后尚未更新的表单，保存一次
///   不再静默重置它没发送的段（此前 `audio.downlink_lead_ms`/`tts.cache_entries`/`[aiui]`
///   就是这样被清掉的）；
/// - 显式 `null` = **清除**该字段 → 反序列化时回落 serde 默认值（这是"缺失=保持"之后
///   无法删除字段的解药）；
/// - 对象**递归**合并，其余类型整体替换。
///
/// ⚠️ 必须在**解析成 `Config` 之前**于 JSON 层完成：`Config` 每个字段都带 serde default，
/// 先解析就无法区分"客户端发了默认值"与"客户端根本没发"。
fn merge_patch(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base.as_object_mut(), patch) {
        (Some(base_obj), serde_json::Value::Object(patch_obj)) => {
            for (k, v) in patch_obj {
                if v.is_null() {
                    base_obj.remove(k); // 清除 → 反序列化取默认值
                } else if let Some(slot) = base_obj.get_mut(k) {
                    merge_patch(slot, v);
                } else {
                    base_obj.insert(k.clone(), v.clone());
                }
            }
        }
        // 基线或补丁不是对象（如 `"tts": 5`）→ 整体替换，随后由反序列化报出精确类型错误
        _ => *base = patch.clone(),
    }
}

/// 保存配置（供管理页面 UI 提交）。写回启动时加载的配置文件（TOML，原注释会丢失）。
///
/// **部分更新语义**：请求里只提到要改的字段即可；未提到的字段保持现盘值（见 [`merge_patch`]）。
/// 热切换能力（TTS，见 [`crate::engine::Engines::refresh_from_disk`]）在新会话开始时按盘上
/// 配置重建，故"保存即对新会话生效"；其余引擎参数（ASR）仍需重启 server。
pub(super) async fn put_config(
    State(engines): State<Arc<Engines>>,
    // 收原始 Value 手动解析：反序列化失败可返回**带精确原因**的 400
    //（Json<Config> 提取器的默认拒绝只有笼统状态码，客户端难定位）。
    Json(raw): Json<serde_json::Value>,
) -> Response {
    if !raw.is_object() {
        return (
            StatusCode::BAD_REQUEST,
            "配置 JSON 必须是对象（如 {\"llm\":{\"model\":\"gpt-4o\"}}）".to_string(),
        )
            .into_response();
    }
    // 0) 过期写入检查：客户端带回上次 GET 的 `revision`（可放 `expected_revision` 或
    //    原样回传 `revision`），与当前盘上内容不一致 → 409 拒绝（而不是覆盖别人的改动）。
    //    未带 revision 的旧客户端跳过检查（向后兼容）。
    let current = current_config(&engines);
    let current_revision = config_revision(&current);
    if let Some(expected) = raw
        .get("expected_revision")
        .or_else(|| raw.get("revision"))
        .and_then(|v| v.as_str())
    {
        if expected != current_revision {
            return (
                StatusCode::CONFLICT,
                json_msg_rev(
                    false,
                    "配置已被其他窗口/端修改，本次保存已取消（避免覆盖别人的改动）。请重新加载配置后再改。",
                    None,
                    Some(&current_revision),
                ),
            )
                .into_response();
        }
    }
    // 1) 现盘配置做基线 → 未出现的字段保持不动；密钥空串 = 保留原值（见 `sanitize_patch`）。
    //    结构性修复：以后新增配置段不会被尚未更新的旧表单清空。
    // 2) 合并结果必须仍是合法 Config：类型/枚举错误在此报 400（带精确原因与字段路径）。
    let body: Config = match merge_client_patch(&current, &raw) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("PUT /api/config：客户端配置 JSON 合并/解析失败: {e}");
            return (StatusCode::BAD_REQUEST, format!("配置 JSON 解析失败: {e}")).into_response();
        }
    };
    let path = match &engines.config_path {
        Some(p) => p.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                json_msg(false, "server 以内置默认配置启动，未指定配置文件，无法持久化。请用 --config 指定 config.toml 后重启。", None),
            )
                .into_response()
        }
    };
    match persist_config(&path, &body) {
        Ok(()) => {
            // 保存成功即回报**实际写入路径**——配置"看不到/丢失"类问题可由此一眼定位
            //（例如容器内路径与宿主机挂载不一致时，Toast 会显示真实路径）。
            let impact = save_impact(&current, &body);
            tracing::debug!(
                "PUT /api/config：本次改动段 [{}]，字段 [{}]，生效档位 {}",
                impact.sections.join(", "),
                impact.paths.join(", "),
                impact.hot.as_str()
            );
            let mut msg = format!(
                "配置已保存到 {path}。TTS/LLM 引擎参数对**新会话**生效，ASR 等需重启 server。"
            );
            if !impact.sections.is_empty() {
                msg.push_str(&format!("本次改动：{}。", impact.sections.join(" / ")));
            }
            if !config_is_persistent(&path) {
                msg.push_str(
                    " ⚠️ 该路径不在挂载卷上：重建容器会丢失配置。请按 docker-compose.yml 挂载 ./server-data:/data（XIAOZHI_CONFIG=/data/config.toml）。",
                );
            }
            // 回传**新** revision：客户端无需再 GET 就能继续保存
            let new_rev = config_revision(&body);
            (
                StatusCode::OK,
                json_msg_full(true, &msg, Some(&path), Some(&new_rev), Some(&impact)),
            )
                .into_response()
        }
        // 健壮性：失败原因同时记入服务端日志（客户端 UI 只能看到状态码）
        Err(e) => {
            tracing::error!("PUT /api/config：写入 {path} 失败: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                json_msg(false, &format!("{e:#}"), Some(&path)),
            )
                .into_response()
        }
    }
}

/// 统一 JSON 响应体（macApp 读取 `message` 显示到 Toast）。
fn json_msg(ok: bool, message: &str, config_path: Option<&str>) -> String {
    json_msg_rev(ok, message, config_path, None)
}

/// 同上，附带 `revision`（客户端用它做乐观并发控制；409 时也回传当前值便于提示）。
fn json_msg_rev(
    ok: bool,
    message: &str,
    config_path: Option<&str>,
    revision: Option<&str>,
) -> String {
    json_msg_full(ok, message, config_path, revision, None)
}

/// 完整响应体：额外带上本次保存的**生效档位**（`hot` / `hot_hint` / `changed_sections`）。
///
/// `hot` 三档 = `live` / `next_session` / `restart_only`，取自注册表字段元数据里最严格的一项；
/// 客户端据此显示「已保存并立即生效 / 已保存，新会话生效 / 已保存，下次启动生效」（§5.4-6）。
fn json_msg_full(
    ok: bool,
    message: &str,
    config_path: Option<&str>,
    revision: Option<&str>,
    impact: Option<&SaveImpact>,
) -> String {
    let mut body = serde_json::json!({
        "ok": ok,
        "message": message,
        "config_path": config_path,
        "revision": revision,
    });
    if let Some(impact) = impact {
        body["hot"] = serde_json::json!(impact.hot.as_str());
        body["hot_hint"] = serde_json::json!(impact.hot.hint());
        body["changed_sections"] = serde_json::json!(impact.sections);
    }
    body.to_string()
}

/// 配置持久化判定：配置文件本身是挂载点（单文件 bind mount），或位于挂载目录之下
///（如 /data/config.toml 落在 ./server-data:/data 卷内）→ 容器重建不丢。
/// 非 Linux（macOS 本地开发）或 /proc 不可读时按"持久"处理（不误报）。
pub(crate) fn config_is_persistent(path: &str) -> bool {
    match std::fs::read_to_string("/proc/self/mountinfo") {
        Ok(mi) => is_persistent_in(&mi, path),
        Err(_) => true,
    }
}

/// mountinfo 解析（独立函数便于单测）：挂载点为目标文件本身，或目标路径位于某挂载目录之下。
fn is_persistent_in(mountinfo: &str, path: &str) -> bool {
    for line in mountinfo.lines() {
        let mut fields = line.split(' ');
        // 格式：id parent major:minor root mount_point options ...
        let _ = fields.next();
        let _ = fields.next();
        let _ = fields.next();
        let _ = fields.next();
        let Some(target) = fields.next() else {
            continue;
        };
        if target == path || path.starts_with(&format!("{target}/")) {
            return true;
        }
    }
    false
}

/// GET /api/config/meta：配置来源元信息（macApp 配置页展示，排查"配置去哪了"）。
pub(super) async fn config_meta(State(engines): State<Arc<Engines>>) -> Response {
    let (path, loaded_from) = match &engines.config_path {
        Some(p) => (Some(p.clone()), "file"),
        None => (None, "default"),
    };
    let persistent = path.as_deref().map(config_is_persistent);
    let hint = match (&path, persistent) {
        (None, _) => "以内置默认配置运行，保存不会持久化（用 --config/XIAOZHI_CONFIG 指定文件后重启）",
        (Some(_), Some(false)) => "⚠️ 配置文件不在挂载卷上：重建容器会丢失（请挂载 ./server-data:/data 并设 XIAOZHI_CONFIG=/data/config.toml）",
        _ => "",
    };
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::json!({
            "config_path": path,
            "loaded_from": loaded_from,
            "persistent": persistent,
            "hint": hint,
        })
        .to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
