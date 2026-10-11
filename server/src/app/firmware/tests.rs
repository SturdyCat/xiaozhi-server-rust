//! `server/src/app/firmware.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。
//!
//! ⚠️ 不碰 `XIAOZHI_FIRMWARE_DIR` 环境变量：它是进程级全局，多个测试并发设置会互相踩。
//! 所有用例都走接收 `root: &Path` 的 `*_in` 版本，各自用独立临时目录。

use super::*;

// ============================================================
// 测试夹具：合成一个 ESP-IDF app 镜像
// ============================================================

fn put_str(buf: &mut [u8], offset: usize, s: &str, len: usize) {
    let n = s.len().min(len - 1);
    buf[offset..offset + n].copy_from_slice(&s.as_bytes()[..n]);
}

/// 合成一个最小 ESP-IDF app 镜像（镜像头 + 段头 + `esp_app_desc_t`）。
fn build_app_image(chip_id: u16, project: &str, version: &str) -> Vec<u8> {
    let mut b = vec![0u8; APP_DESC_OFFSET + APP_DESC_LEN];
    b[0] = 0xE9; // esp_image_header_t.magic
    b[CHIP_ID_OFFSET..CHIP_ID_OFFSET + 2].copy_from_slice(&chip_id.to_le_bytes());
    b[APP_DESC_OFFSET..APP_DESC_OFFSET + 4].copy_from_slice(&APP_DESC_MAGIC.to_le_bytes());
    put_str(&mut b, APP_DESC_OFFSET + 16, version, 32);
    put_str(&mut b, APP_DESC_OFFSET + 48, project, 32);
    put_str(&mut b, APP_DESC_OFFSET + 80, "10:20:30", 16);
    put_str(&mut b, APP_DESC_OFFSET + 96, "Oct 11 2026", 16);
    put_str(&mut b, APP_DESC_OFFSET + 112, "v5.4.1", 32);
    b
}

/// 独立临时根目录（测试并发安全：目录名带用例名）。
fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xz_fw_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 在临时根目录里落一个托管条目（bin + meta.json）。
fn write_entry(root: &Path, project: &str, model: &str, board: &str, version: &str, uploaded_at: u64) {
    let dir = entry_dir(root, project, model, board);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(BIN_FILENAME), b"fake-bin").unwrap();
    let meta = FirmwareMeta {
        project: project.into(),
        model: model.into(),
        board: board.into(),
        version: version.into(),
        size: 8,
        sha256: "deadbeef".into(),
        idf_ver: "v5.4.1".into(),
        built_at: "Oct 11 2026 10:20:30".into(),
        uploaded_at,
    };
    std::fs::write(
        dir.join(META_FILENAME),
        serde_json::to_string(&meta).unwrap(),
    )
    .unwrap();
}

// ============================================================
// 镜像元数据解析
// ============================================================

/// 包内元数据自动识别：型号来自镜像头 chip_id、功能/版本来自 app 描述符。
#[test]
fn parse_app_image_reads_chip_and_app_desc() {
    let bin = build_app_image(0x0009, "xiaozhi", "2.0.4");
    let info = parse_app_image(&bin).expect("ESP-IDF 镜像应能解析");
    assert_eq!(info.model, "esp32s3", "chip_id 9 = esp32s3（与 CONFIG_IDF_TARGET 同名）");
    assert_eq!(info.project, "xiaozhi");
    assert_eq!(info.version, "2.0.4");
    assert_eq!(info.idf_ver, "v5.4.1");
    assert_eq!(info.built_at, "Oct 11 2026 10:20:30");
    // c3 同样可识别
    assert_eq!(
        parse_app_image(&build_app_image(0x0005, "xiaozhi", "2.0.4")).unwrap().model,
        "esp32c3"
    );
    // 未知芯片 id：型号留空（调用方要求显式给 ?model=，绝不猜）
    assert_eq!(parse_app_image(&build_app_image(0x00FF, "x", "1.0.0")).unwrap().model, "");
}

/// 非 ESP-IDF 镜像（魔数不符/长度不足）→ None，由上传处理器回退到查询参数。
#[test]
fn parse_app_image_rejects_non_esp_image() {
    assert!(parse_app_image(&[]).is_none());
    assert!(parse_app_image(&vec![0u8; APP_DESC_OFFSET + APP_DESC_LEN]).is_none(), "全 0 无魔数");
    assert!(parse_app_image(&build_app_image(0x0009, "xiaozhi", "2.0.4")[..64]).is_none(), "长度不足");
}

/// chip_id → 型号名必须与固件 `SystemInfo::GetChipModelName()`（`CONFIG_IDF_TARGET`）一致，
/// 否则设备上报的 `chip_model_name` 永远匹配不上托管条目。
#[test]
fn chip_model_names_match_firmware_targets() {
    assert_eq!(chip_model_name(0x0000), Some("esp32"));
    assert_eq!(chip_model_name(0x0002), Some("esp32s2"));
    assert_eq!(chip_model_name(0x0005), Some("esp32c3"));
    assert_eq!(chip_model_name(0x0009), Some("esp32s3"));
    assert_eq!(chip_model_name(0x000C), Some("esp32c2"));
    assert_eq!(chip_model_name(0x000D), Some("esp32c6"));
    assert_eq!(chip_model_name(0x0012), Some("esp32p4"));
    assert_eq!(chip_model_name(0xFFFF), None, "未知芯片不猜型号");
}

// ============================================================
// 版本比较（语义对齐固件 Ota::IsNewVersionAvailable）
// ============================================================

#[test]
fn is_newer_follows_firmware_semantics() {
    assert!(is_newer("2.0.4", "2.0.3"));
    assert!(is_newer("2.1.0", "2.0.9"));
    assert!(is_newer("1.0.0.1", "1.0.0"), "前缀相同则段数多者更新");
    assert!(!is_newer("2.0.4", "2.0.4"), "同版本不算更新");
    assert!(!is_newer("2.0.3", "2.0.4"), "旧版本不算更新");
    assert!(!is_newer("1.0.0", "1.0.0.1"));
    // 不可解析：一律 false（畸形版本号不得触发降级/无限升级）
    assert!(!is_newer("2.0.4", "1.6.4-abc"));
    assert!(!is_newer("v2.0.4", "1.0.0"));
    assert!(!is_newer("2.0.4", ""));
    assert!(!is_newer("", "1.0.0"));
}

// ============================================================
// 路径段净化
// ============================================================

#[test]
fn sanitize_segment_blocks_path_traversal() {
    assert_eq!(sanitize_segment(" 2.0.4 "), Some("2.0.4".into()));
    assert_eq!(sanitize_segment("esp32s3"), Some("esp32s3".into()));
    assert_eq!(sanitize_segment("bread-compact-wifi"), Some("bread-compact-wifi".into()));
    // 路径穿越 / 注入：一律拒绝（会拼进存储目录与下载 URL）
    assert_eq!(sanitize_segment("../etc/passwd"), None);
    assert_eq!(sanitize_segment(".."), None, "纯点段会穿透目录");
    assert_eq!(sanitize_segment("."), None);
    assert_eq!(sanitize_segment("a/b"), None);
    assert_eq!(sanitize_segment("a?b=1"), None);
    assert_eq!(sanitize_segment(""), None);
    assert_eq!(sanitize_segment(&"a".repeat(65)), None, "超长拒绝");
}

// ============================================================
// 匹配：同功能 + 同型号，板型精确优先，绝不跨型号/跨功能
// ============================================================

#[test]
fn find_requires_same_project_and_model_and_prefers_board() {
    let root = temp_root("find_match");
    write_entry(&root, "xiaozhi", "esp32s3", "", "2.0.4", 100);
    write_entry(&root, "xiaozhi", "esp32s3", "bread-compact-wifi", "2.0.5", 90);
    write_entry(&root, "xiaozhi", "esp32c3", "", "2.0.6", 110);
    write_entry(&root, "otherapp", "esp32s3", "", "9.0.0", 120);

    // 板型精确匹配优先（即使通用条目更新）
    let m = find_in(&root, "xiaozhi", "esp32s3", "bread-compact-wifi").unwrap();
    assert_eq!(m.version, "2.0.5");
    assert_eq!(m.board, "bread-compact-wifi");
    // 板型未命中 → 回落到"该型号通用"条目
    let m = find_in(&root, "xiaozhi", "esp32s3", "lichuang-dev").unwrap();
    assert_eq!(m.version, "2.0.4");
    assert_eq!(m.board, "");
    // 同功能不同型号：互不相干（c3 不会拿到 s3 的包）
    assert_eq!(find_in(&root, "xiaozhi", "esp32c3", "").unwrap().version, "2.0.6");
    assert!(find_in(&root, "xiaozhi", "esp32s2", "").is_none());
    // 同型号不同功能：互不相干
    assert_eq!(find_in(&root, "otherapp", "esp32s3", "").unwrap().version, "9.0.0");
    assert!(find_in(&root, "otherapp", "esp32c3", "").is_none());
    // 没有型号信息（老固件不上报）→ 不下发，绝不猜
    assert!(find_in(&root, "", "", "").is_none());
    assert!(find_in(&root, "xiaozhi", "", "").is_none());
    // 全量扫描：4 条
    assert_eq!(scan_in(&root).len(), 4);
    // 二进制路径按 功能/型号/板型 分目录
    let m = find_in(&root, "xiaozhi", "esp32s3", "lichuang-dev").unwrap();
    assert_eq!(bin_path_in(&root, &m), root.join("xiaozhi/esp32s3/_/firmware.bin"));
}

/// 上传即覆盖：同 key 只有一份；扫描结果按上传时间倒序（新的在前）。
#[test]
fn scan_lists_newest_first_and_single_entry_per_key() {
    let root = temp_root("scan_order");
    write_entry(&root, "xiaozhi", "esp32c3", "", "2.0.1", 100);
    write_entry(&root, "xiaozhi", "esp32s3", "", "2.0.2", 300);
    write_entry(&root, "xiaozhi", "esp32s3", "x", "2.0.3", 200);
    let all = scan_in(&root);
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].version, "2.0.2", "新上传的排前面");
}

// ============================================================
// 删除 / 旧版条目
// ============================================================

#[test]
fn remove_deletes_entry_and_cleans_empty_parents() {
    let root = temp_root("remove_entry");
    write_entry(&root, "xiaozhi", "esp32c3", "", "2.0.1", 100);
    assert!(remove_in(&root, "xiaozhi", "esp32c3", ""));
    assert!(find_in(&root, "xiaozhi", "esp32c3", "").is_none());
    assert!(!root.join("xiaozhi/esp32c3").exists(), "空掉的父目录一并清理");
    assert!(!remove_in(&root, "xiaozhi", "esp32c3", ""), "重复删除返回 false");
    assert!(root.exists(), "根目录保留");
}

/// 旧版单固件（根目录 firmware.bin + latest.json）：可列出、可删除，但**不参与匹配**
///（它没有型号信息，继续下发等于把不确定的包发给任何设备）。
#[test]
fn legacy_root_entry_is_listed_but_never_matches() {
    let root = temp_root("legacy");
    std::fs::write(root.join(BIN_FILENAME), b"old-bin").unwrap();
    std::fs::write(
        root.join("latest.json"),
        serde_json::json!({"version": "1.9.0", "size": 7, "sha256": "aa", "uploaded_at": 42}).to_string(),
    )
    .unwrap();

    let all = scan_in(&root);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].project, LEGACY);
    assert_eq!(all[0].version, "1.9.0");
    assert!(all[0].to_json()["legacy"].as_bool().unwrap());
    // 即便有人拿 _legacy 当功能/型号来查，也不匹配
    assert!(find_in(&root, LEGACY, LEGACY, "").is_none());
    // 路径落在根目录
    assert_eq!(bin_path_in(&root, &all[0]), root.join(BIN_FILENAME));
    // 删除旧版条目 = 删根目录两文件
    assert!(remove_in(&root, LEGACY, LEGACY, ""));
    assert!(!root.join(BIN_FILENAME).exists());
    assert!(!root.join("latest.json").exists());
    assert!(scan_in(&root).is_empty());
}
