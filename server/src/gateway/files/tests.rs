//! 文件管理回归（纯函数部分）：路径校验 / 受保护名 / 解析防外逃 / 列表排序 / 操作契约。
//! 真实 loopback HTTP 回归在 [http_tests.rs](<http_tests.rs>)；共享夹具（`temp_root`/
//! `canonical`/`json`）在本文件，供兄弟测试模块 `super::tests::` 复用。
//! 全部落盘在 `std::env::temp_dir` 下的独立目录，互不干扰、不触生产数据。

use super::*;

/// 独立临时目录（先清后用；各用例 tag 不同可并行）。
pub(super) fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xz-files-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// canonical 根的展示串（macOS 上 canonicalize 给 `/private/var/...`，与
/// `root.display()` 的 `/var/...` 不同，断言一律以 canonical 结果为准）。
pub(super) fn canonical(root: &Path) -> String {
    std::fs::canonicalize(root).unwrap().display().to_string()
}

/// body 字节 → JSON。
pub(super) fn json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap()
}

// ============================================================
// 纯函数：路径校验
// ============================================================

#[test]
fn validate_rel_accepts_normal_paths_and_trims_trailing_slash() {
    for (raw, want) in [
        ("", ""),
        ("/", ""),
        ("a", "a"),
        ("a/b", "a/b"),
        ("a/b/", "a/b"),
        ("oww-models/hey_biscuit.onnx", "oww-models/hey_biscuit.onnx"),
        ("中文 目录/模型.onnx", "中文 目录/模型.onnx"),
        ("  a/b  ", "a/b"),
        ("a b/c-d.e_f", "a b/c-d.e_f"),
        (".hidden/.onnx", ".hidden/.onnx"), // 点开头的普通文件名不是 `.` 段
    ] {
        assert_eq!(validate_rel(raw).unwrap(), want, "raw = {raw:?}");
    }
}

#[test]
fn validate_rel_rejects_traversal_and_odd_segments() {
    for raw in [
        "..", "../x", "a/..", "a/../b", ".", "a/.", "./a", "/a", "a//b", "a\\b", "a/b\\", "a\0b",
    ] {
        assert!(validate_rel(raw).is_err(), "{raw:?} 应被拒绝");
    }
}

// ============================================================
// 纯函数：受保护名
// ============================================================

#[test]
fn protected_names_match_final_component_at_any_depth() {
    for (rel, want) in [
        ("data.db", true),
        ("data.db-wal", true),
        ("data.db-shm", true),
        ("a/b/data.db", true),
        ("oww-models/data.db", true),
        ("data.db2", false),
        ("xdata.db", false),
        ("data.dbx", false),
        ("a/data.db/hey.onnx", false),
        ("", false),
    ] {
        assert_eq!(is_protected(rel), want, "rel = {rel:?}");
    }
    assert!(
        protected_reason().contains("配置/记忆库"),
        "403 文案要说明原因"
    );
}

// ============================================================
// 纯函数：解析（符号链接防外逃）
// ============================================================

#[test]
fn resolve_in_blocks_symlink_escape_and_accepts_missing_targets() {
    let root = temp_root("resolve");
    let outside = temp_root("resolve-outside");
    std::fs::create_dir_all(root.join("sub/deep")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), b"outside").unwrap();

    // 已存在目标 → canonical 结果
    assert_eq!(
        resolve_in(&root, "sub/deep"),
        Some(std::fs::canonicalize(root.join("sub/deep")).unwrap())
    );
    // 根本身
    assert_eq!(
        resolve_in(&root, ""),
        Some(std::fs::canonicalize(&root).unwrap())
    );
    // 目标不存在（upload/mkdir 场景）→ 最近存在祖先校验后拼回
    assert!(resolve_in(&root, "sub/deep/new.bin").is_some());
    // 根不存在 → None
    assert_eq!(resolve_in(&temp_root("resolve-missing-root"), "a"), None);

    // 符号链接外逃：链接自身、链接下的文件都拒绝
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("ln")).unwrap();
        assert_eq!(resolve_in(&root, "ln"), None);
        assert_eq!(resolve_in(&root, "ln/secret.txt"), None);
    }
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
}

// ============================================================
// 纯函数：列表排序与字段
// ============================================================

#[test]
fn list_entries_sorts_dirs_first_then_name_with_fields() {
    let root = temp_root("list");
    std::fs::create_dir_all(root.join("dir_b")).unwrap();
    std::fs::create_dir_all(root.join("DirA")).unwrap();
    std::fs::write(root.join("b.txt"), b"12345").unwrap();
    std::fs::write(root.join("a.onnx"), b"abc").unwrap();
    std::fs::write(root.join("data.db"), b"db").unwrap();

    let entries = list_entries(&root).unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    // 目录在前（名称序），文件随后（名称序）；排序对保护名一视同仁
    assert_eq!(names, ["DirA", "dir_b", "a.onnx", "b.txt", "data.db"]);
    let a = entries.iter().find(|e| e.name == "a.onnx").unwrap();
    assert!(!a.dir);
    assert_eq!(a.size, 3);
    assert!(!a.protected);
    assert!(a.modified > 0, "modified 应为 unix 秒");
    assert!(entries.iter().find(|e| e.name == "DirA").unwrap().dir);
    assert!(
        entries
            .iter()
            .find(|e| e.name == "data.db")
            .unwrap()
            .protected
    );
    assert_eq!(
        entries.iter().find(|e| e.name == "DirA").unwrap().size,
        0,
        "目录 size 恒 0"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ============================================================
// 操作契约矩阵
// ============================================================

#[test]
fn op_upload_matrix_new_conflict_overwrite_dir_and_protected() {
    let root = temp_root("op-upload");
    // 新文件 + 自动建父目录
    assert_eq!(
        op_upload(&root, "oww-models/hey.onnx", b"abc", false).unwrap(),
        3
    );
    assert_eq!(
        std::fs::read(root.join("oww-models/hey.onnx")).unwrap(),
        b"abc"
    );
    // 已存在未 overwrite → 409
    let e = op_upload(&root, "oww-models/hey.onnx", b"xy", false).unwrap_err();
    assert_eq!(e.status, StatusCode::CONFLICT);
    assert!(
        e.message.contains("overwrite"),
        "409 要提示怎么覆盖: {}",
        e.message
    );
    // overwrite=1 → 覆盖
    assert_eq!(
        op_upload(&root, "oww-models/hey.onnx", b"xyz9", true).unwrap(),
        4
    );
    assert_eq!(
        std::fs::read(root.join("oww-models/hey.onnx")).unwrap(),
        b"xyz9"
    );
    // 目标是目录 → 409（带 overwrite 也一样）
    std::fs::create_dir_all(root.join("d")).unwrap();
    for ow in [false, true] {
        assert_eq!(
            op_upload(&root, "d", b"z", ow).unwrap_err().status,
            StatusCode::CONFLICT
        );
    }
    // 受保护名：新建与覆盖都 403（受保护检查优先于存在性检查，父目录也不应被创建）
    for rel in ["data.db", "sub/data.db-wal", "data.db-shm"] {
        for ow in [false, true] {
            let e = op_upload(&root, rel, b"z", ow).unwrap_err();
            assert_eq!(e.status, StatusCode::FORBIDDEN, "rel={rel}");
            assert!(e.message.contains("配置/记忆库"), "{}", e.message);
        }
    }
    assert!(!root.join("sub").exists(), "受保护检查应先于父目录创建");
    // 根目录不可作为上传目标
    assert_eq!(
        op_upload(&root, "", b"z", false).unwrap_err().status,
        StatusCode::BAD_REQUEST
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn op_delete_matrix_file_dir_recursive_protected_and_missing() {
    let root = temp_root("op-delete");
    std::fs::create_dir_all(root.join("d1/d2")).unwrap();
    std::fs::write(root.join("d1/d2/deep.txt"), b"x").unwrap();
    std::fs::write(root.join("f.txt"), b"x").unwrap();
    std::fs::write(root.join("data.db"), b"db").unwrap();

    // 文件
    op_delete(&root, "f.txt").unwrap();
    assert!(!root.join("f.txt").exists());
    // 再删 → 404
    assert_eq!(
        op_delete(&root, "f.txt").unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    // 目录递归
    op_delete(&root, "d1").unwrap();
    assert!(!root.join("d1").exists());
    // 受保护：真实存在与否都 403（含任意层级）
    for rel in ["data.db", "a/b/data.db-shm"] {
        let e = op_delete(&root, rel).unwrap_err();
        assert_eq!(e.status, StatusCode::FORBIDDEN, "rel={rel}");
        assert!(e.message.contains("配置/记忆库"), "{}", e.message);
    }
    assert!(root.join("data.db").exists(), "受保护文件不能被删");
    // 根 → 400；缺失 → 404
    assert_eq!(
        op_delete(&root, "").unwrap_err().status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        op_delete(&root, "nope").unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn op_list_optional_dir_missing_is_empty_200() {
    let root = temp_root("op-list-opt");
    std::fs::create_dir_all(&root).unwrap();

    // `oww-models/` 未创建（没人传过唤醒词模型）：空清单 200，而不是 404——
    // 管理台每次启动都会无条件查它，404 会变成一条假的客户端错误日志。
    let (listed_root, entries) = op_list(&root, OWW_MODELS_DIR).unwrap();
    assert_eq!(listed_root, std::fs::canonicalize(&root).unwrap());
    assert!(entries.is_empty(), "缺失的可选业务目录应回空清单");

    // 白名单只认顶层精确名：其它缺失路径、以及它的子路径仍 404。
    assert_eq!(
        op_list(&root, "missing").unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        op_list(&root, "oww-models/sub").unwrap_err().status,
        StatusCode::NOT_FOUND
    );

    // 目录建起来后照常列出内容（上传/建目录仍是按需创建，不预创建）。
    op_mkdir(&root, OWW_MODELS_DIR).unwrap();
    std::fs::write(root.join(OWW_MODELS_DIR).join("a.onnx"), b"x").unwrap();
    let (_, entries) = op_list(&root, OWW_MODELS_DIR).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a.onnx");

    // 存储根本身不存在时仍是 404（首次部署/卷未挂载），行为不变。
    let absent = root.join("no-such-root");
    assert_eq!(
        op_list(&absent, OWW_MODELS_DIR).unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn op_download_and_op_list_contract_edges() {
    let root = temp_root("op-dl");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/f.txt"), b"hello").unwrap();

    let (name, bytes) = op_download(&root, "sub/f.txt").unwrap();
    assert_eq!(name, "f.txt");
    assert_eq!(bytes, b"hello");
    assert_eq!(
        op_download(&root, "sub").unwrap_err().status,
        StatusCode::BAD_REQUEST,
        "目录不能下载"
    );
    assert_eq!(
        op_download(&root, "missing").unwrap_err().status,
        StatusCode::NOT_FOUND
    );

    // 列表：返回 canonical 根 + 条目；缺失 404；文件路径 400
    let (listed_root, entries) = op_list(&root, "sub").unwrap();
    assert_eq!(listed_root, std::fs::canonicalize(&root).unwrap());
    assert_eq!(entries.len(), 1);
    assert_eq!(
        op_list(&root, "missing").unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        op_list(&root, "sub/f.txt").unwrap_err().status,
        StatusCode::BAD_REQUEST
    );
    // 建目录：幂等 / 同名文件 409 / 根 400
    op_mkdir(&root, "x/y").unwrap();
    op_mkdir(&root, "x/y").unwrap();
    assert!(root.join("x/y").is_dir());
    std::fs::write(root.join("n.txt"), b"").unwrap();
    assert_eq!(
        op_mkdir(&root, "n.txt").unwrap_err().status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        op_mkdir(&root, "").unwrap_err().status,
        StatusCode::BAD_REQUEST
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn router_builds_without_path_conflicts() {
    // axum 在路由路径冲突时 panic
    let _ = router();
}
