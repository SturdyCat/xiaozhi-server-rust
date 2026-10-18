//! [`super::nlu_build`] 测试：端点纯逻辑部分。
//!
//! docker 真训练不在单测范围（同 sherpa 口径：单测桩不冒充可运行服务）；
//! 这里验证：前置校验拒绝口径、状态端点形状、日志尾部截断（UTF-8 边界安全）、
//! docker.sock 挂载路径的宿主侧换算。

use super::*;

#[test]
fn validate_rejects_when_nlu_disabled() {
    let err = validate_buildable(&crate::plugins::nlu::NluConfig::default()).unwrap_err();
    assert!(err.contains("enabled=false"), "{err}");
}

#[test]
fn validate_rejects_when_no_intents() {
    let mut nlu = crate::plugins::nlu::NluConfig::default();
    nlu.enabled = true;
    let err = validate_buildable(&nlu).unwrap_err();
    assert!(err.contains("intents 为空"), "{err}");
}

#[test]
fn validate_accepts_builtin_reminders() {
    let mut nlu = crate::plugins::nlu::NluConfig::default();
    nlu.enabled = true;
    // 内置提醒技能合并后即使配置零意图也可训练。
    assert!(validate_buildable(&nlu.with_builtins()).is_ok());
}

#[tokio::test]
async fn status_body_shape() {
    let res = build_status().await;
    let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["state"], "idle");
    assert_eq!(body["running"], false);
    assert!(body["expected_config"]["bert_enabled"].as_bool().unwrap());
}

#[tokio::test]
async fn push_log_keeps_tail_only_and_utf8_safe() {
    // push_log 现自带锁（训练期间不得跨子进程持锁），测试也走异步入口。
    push_log(&"训".repeat(6 * 1024)).await; // 多字节字符，验证边界回退
    push_log(&"x".repeat(20 * 1024)).await;
    let job = job().lock().await;
    assert!(job.log.len() <= 8 * 1024 + 32);
    assert!(job.log.is_char_boundary(0));
    assert!(job.log.ends_with('\n'));
}

#[test]
fn mount_host_path_bare_metal_unchanged() {
    // 无 mountinfo 匹配（裸机或非容器路径）：原样返回。
    assert_eq!(mount_host_path("/data/nlu-train", ""), "/data/nlu-train");
}

#[test]
fn mount_host_path_maps_container_data_bind() {
    // compose：宿主 ./server-data → 容器 /data（bind 挂载源为绝对路径）。
    let info = "721 719 0:99 / /data rw,relatime - ext4 /srv/xiaozhi/server-data rw\n";
    assert_eq!(
        mount_host_path("/data/nlu-train", info),
        "/srv/xiaozhi/server-data/nlu-train"
    );
    assert_eq!(
        mount_host_path("/data", info),
        "/srv/xiaozhi/server-data"
    );
}

#[test]
fn mount_host_path_prefers_longest_match() {
    // 嵌套挂载取最长 mountpoint 前缀。
    let info = concat!(
        "10 1 0:5 / /data rw - ext4 /mnt/host-data rw\n",
        "11 10 0:6 / /data/models rw - nfs 10.0.0.9:/exports/models rw\n",
    );
    assert_eq!(
        mount_host_path("/data/models/nlu", info),
        "10.0.0.9:/exports/models/nlu"
    );
    assert_eq!(
        mount_host_path("/data/nlu-train", info),
        "/mnt/host-data/nlu-train"
    );
}

#[test]
fn mount_host_path_ignores_prefix_collision() {
    // /data 不应匹配 /database。
    let info = "10 1 0:5 / /database rw - ext4 /mnt/other rw\n";
    assert_eq!(mount_host_path("/data/nlu-train", info), "/data/nlu-train");
}

#[test]
fn mount_host_path_subtree_root() {
    // 挂载源端是子树（root 非 /，如镜像分层 bind）：定位到子树内同相对位置。
    let info = "10 1 0:5 /server-data /data rw - ext4 /mnt/host rw\n";
    assert_eq!(
        mount_host_path("/data/nlu-train", info),
        "/mnt/host/server-data/nlu-train"
    );
}

#[test]
fn mount_host_path_skips_longer_mountpoint_without_panic() {
    // 真实事故：mountinfo 里存在比目标路径更长的兄弟挂载点，
    // 旧实现先按 mnt.len() 切片再校验前缀，直接 byte index 越界 panic。
    let info = concat!(
        "10 1 0:5 / /data rw - ext4 /mnt/host-data rw\n",
        "12 10 0:7 / /data/nlu-train-env rw - ext4 /mnt/host-data/nlu-train-env rw\n",
    );
    assert_eq!(
        mount_host_path("/data/nlu-train", info),
        "/mnt/host-data/nlu-train"
    );
}

#[test]
fn map_via_mounts_replaces_longest_destination() {
    // 真实事故（UGOS NAS）：mountinfo source 只是设备名，需走自己容器 inspect
    // 的 Mounts 做精确映射；嵌套挂载取最长 Destination。
    let mounts = vec![
        (
            "/data".to_string(),
            "/volume3/docker/xiaozhi-server-rust/server-data".to_string(),
        ),
        (
            "/data/nlu-train-env".to_string(),
            "/volume3/docker/xiaozhi-server-rust/server-data/nlu-train-env".to_string(),
        ),
    ];
    assert_eq!(
        map_via_mounts("/data/nlu-train", &mounts),
        Some("/volume3/docker/xiaozhi-server-rust/server-data/nlu-train".to_string())
    );
}

#[test]
fn map_via_mounts_rejects_non_boundary_prefix() {
    // /database 不是 /data 的挂载点，不能只按字符串前缀命中。
    let mounts = vec![("/database".to_string(), "/host/db".to_string())];
    assert_eq!(map_via_mounts("/data/nlu-train", &mounts), None);
}
