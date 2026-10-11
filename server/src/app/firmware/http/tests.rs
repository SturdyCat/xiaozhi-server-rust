//! HTTP 层单测：上传坐标解析（纯函数）、路由构造、只读端点的响应形状。
//!
//! ⚠️ 上传/删除处理器需要 `Arc<Engines>`（构造它会做必需能力校验，未启用 `sherpa` 的
//! 测试环境起不来），因此这里覆盖的是三件真正容易写错的事：
//! 1. `resolve_target`——**查询参数 vs 包内元数据**的优先级与缺字段时的可行动报错；
//! 2. `router()` 构造——axum 在路径冲突时 panic（`/api/ota/firmware` 与 `/latest`、`/delete` 共存）；
//! 3. 只读端点 `list`/`download_latest` 的**实际 HTTP 响应**（状态码 + body + Content-Length）。

use super::*;
use axum::body::to_bytes;

fn info(project: &str, model: &str, version: &str) -> AppImageInfo {
    AppImageInfo {
        project: project.into(),
        model: model.into(),
        version: version.into(),
        idf_ver: "v5.4.1".into(),
        built_at: "Oct 11 2026".into(),
    }
}

fn q(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn resolve_target_prefers_query_over_package_metadata() {
    let pkg = info("xiaozhi", "esp32s3", "2.0.4");

    // 全留空 → 完全采用包内元数据（型号/版本自动识别，管理员无需手填）
    let (p, m, b, v) = resolve_target(&q(&[]), Some(&pkg)).unwrap();
    assert_eq!(
        (p.as_str(), m.as_str(), b.as_str(), v.as_str()),
        ("xiaozhi", "esp32s3", "", "2.0.4")
    );

    // 显式覆盖优先（人工改功能/版本/板型；板型是"收窄到具体板"的唯一入口）
    let (p, m, b, v) = resolve_target(
        &q(&[
            ("project", "otherapp"),
            ("version", "9.9.9"),
            ("board", "bread-compact-wifi"),
        ]),
        Some(&pkg),
    )
    .unwrap();
    assert_eq!(
        (p.as_str(), m.as_str(), b.as_str(), v.as_str()),
        ("otherapp", "esp32s3", "bread-compact-wifi", "9.9.9")
    );

    // 空串/空白 = 未给（回落包内元数据），不是"清空"
    let (_, _, _, v) = resolve_target(&q(&[("version", "   ")]), Some(&pkg)).unwrap();
    assert_eq!(v, "2.0.4");
}

#[test]
fn resolve_target_requires_all_three_and_says_how_to_fix() {
    // 非 ESP-IDF 镜像（解析失败）且没给查询参数 → 报错必须说清"怎么补"
    let e = resolve_target(&q(&[("model", "esp32c3")]), None).unwrap_err();
    assert!(e.contains("?project=&model=&version="), "报错要可行动: {e}");
    assert!(e.contains("解析失败"), "要说清是解析失败还是字段为空: {e}");

    // 包内元数据存在但字段为空 → 另一条文案
    let e = resolve_target(&q(&[]), Some(&info("", "", ""))).unwrap_err();
    assert!(e.contains("字段为空"), "{e}");
}

#[test]
fn resolve_target_rejects_path_traversal_board() {
    let e = resolve_target(
        &q(&[
            ("project", "xiaozhi"),
            ("model", "esp32s3"),
            ("version", "1.0.0"),
            ("board", "../etc"),
        ]),
        None,
    )
    .unwrap_err();
    assert!(e.contains("board 非法"), "{e}");
}

#[test]
fn router_builds_without_path_conflicts() {
    // axum 在路由路径冲突时 panic（例如把 `/latest` 写成会被 `/api/ota/firmware` 吞掉的形式）
    let _ = router();
}

/// 只读端点走 `firmware_dir()`（环境变量 `XIAOZHI_FIRMWARE_DIR`）。
/// 环境变量是**进程级**的：所有依赖它的断言都放在这一个用例里，避免并行测试互相覆盖。
#[tokio::test]
async fn read_endpoints_serve_scan_and_download() {
    let root = std::env::temp_dir().join(format!("xz-fw-http-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::env::set_var("XIAOZHI_FIRMWARE_DIR", &root);

    let dir = entry_dir(&root, "xiaozhi", "esp32s3", "");
    std::fs::create_dir_all(&dir).unwrap();
    write_atomic(&dir.join(BIN_FILENAME), b"BINARY").unwrap();
    let meta = FirmwareMeta {
        project: "xiaozhi".into(),
        model: "esp32s3".into(),
        board: String::new(),
        version: "2.0.4".into(),
        size: 6,
        sha256: "ab".repeat(32),
        idf_ver: "v5.4.1".into(),
        built_at: "Oct 11 2026".into(),
        uploaded_at: 1,
    };
    write_atomic(
        &dir.join(META_FILENAME),
        serde_json::to_string(&meta).unwrap().as_bytes(),
    )
    .unwrap();

    // 清单：管理页据此渲染；dir 要回真实目录（"固件存哪了"直接可见）
    let resp = list().await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["count"], 1);
    assert_eq!(v["entries"][0]["model"], "esp32s3");
    assert_eq!(v["entries"][0]["version"], "2.0.4");
    assert_eq!(v["dir"], root.display().to_string());

    // 命中：裸 bin + Content-Length（设备侧 Ota::Upgrade 依赖它，缺失即判定失败）
    let resp = download_latest(Query(q(&[("project", "xiaozhi"), ("model", "esp32s3")]))).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_LENGTH).unwrap(),
        "6",
        "必须给出非零 Content-Length"
    );
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/octet-stream"
    );
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], b"BINARY");

    // 型号不符：404 且文案指路（设备日志里唯一能看到的东西）
    let resp = download_latest(Query(q(&[("project", "xiaozhi"), ("model", "esp32c3")]))).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("/api/ota/firmware"),
        "404 文案要指路"
    );

    std::env::remove_var("XIAOZHI_FIRMWARE_DIR");
    let _ = std::fs::remove_dir_all(&root);
}
