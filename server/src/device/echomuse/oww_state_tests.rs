//! `/oww/state` 五态判定回归（纯函数，不连设备、不下发任何字节）。
use super::*;

/// 五态逐一钉死：**读不到设备 ≠ 缺失**（否则管理台会催"重装"，而设备只是 shell 面
/// 还没起来）。
#[test]
fn asset_state_covers_all_five_states_without_faking_absence() {
    let want = "aa".repeat(32);
    let other = "cc".repeat(32);
    let mut files = HashMap::new();
    files.insert(VAD_NAME.to_string(), want.clone());
    files.insert(ORT_SO_FILE.to_string(), "bb".repeat(32));
    let full = DeviceOwwState {
        files,
        free_mb: Some(120),
    };
    let empty = DeviceOwwState {
        files: HashMap::new(),
        free_mb: Some(120),
    };

    // installed：摘要一致（大小写不敏感——设备侧 sha256sum 输出小写，别因大小写误判旧版）
    assert_eq!(
        asset_state(Some(&full.files), VAD_NAME, Some(&want), Some(120)),
        AssetState::Installed
    );
    assert_eq!(
        asset_state(
            Some(&full.files),
            VAD_NAME,
            Some(&want.to_uppercase()),
            Some(120)
        ),
        AssetState::Installed
    );

    // outdated：设备有一份，但摘要与服务端的不同
    assert_eq!(
        asset_state(Some(&full.files), VAD_NAME, Some(&other), Some(120)),
        AssetState::Outdated
    );

    // missing：清单读到了，就是没这个文件，空间够
    assert_eq!(
        asset_state(Some(&empty.files), VAD_NAME, Some(&want), Some(120)),
        AssetState::Missing
    );

    // blocked：缺这件且剩余空间低于官方下限
    assert_eq!(
        asset_state(
            Some(&empty.files),
            VAD_NAME,
            Some(&want),
            Some(FREE_FLOOR_MB - 1)
        ),
        AssetState::Blocked
    );
    // 恰好等于下限不算 blocked（官方语义是"低于"才拒推）
    assert_eq!(
        asset_state(
            Some(&empty.files),
            VAD_NAME,
            Some(&want),
            Some(FREE_FLOOR_MB)
        ),
        AssetState::Missing
    );

    // unknown ①：读不到设备（shell 面没起来 / 会话占用）——绝不能报 missing
    assert_eq!(
        asset_state(None, VAD_NAME, Some(&want), None),
        AssetState::Unknown
    );
    // unknown ②：服务端自己没有可比对的资产（未下载/不可解析）
    assert_eq!(
        asset_state(Some(&full.files), VAD_NAME, None, Some(120)),
        AssetState::Unknown
    );

    // 对外字面量闭合且顺序固定（前端按这五个值做映射）
    assert_eq!(
        [
            AssetState::Installed.as_str(),
            AssetState::Outdated.as_str(),
            AssetState::Blocked.as_str(),
            AssetState::Unknown.as_str(),
            AssetState::Missing.as_str(),
        ],
        ASSET_STATES
    );
}

/// 回包形状：缺失一律 null、`states` 自描述五态、读不到设备时 state=unknown。
#[test]
fn payload_reports_nulls_and_closed_state_set() {
    let device = DeviceOwwState {
        files: HashMap::new(),
        free_mb: None,
    };
    let body = oww_state_payload("d1", Some(&device), Some(("aa", 42)), Some("bb"), 1234);
    assert_eq!(body["device_id"], "d1");
    assert_eq!(body["vad"]["state"], "missing");
    assert_eq!(body["vad"]["file"], VAD_NAME);
    assert_eq!(body["vad"]["sha256"], "aa");
    assert_eq!(body["vad"]["bytes"], 42);
    assert_eq!(body["ort"]["state"], "missing");
    assert_eq!(body["ort"]["file"], ORT_SO_FILE);
    assert_eq!(body["ort"]["sha256"], "bb");
    assert!(body["free_mb"].is_null(), "读不到空间必须是 null，不是 0");
    assert_eq!(body["checked_at"], 1234);
    assert_eq!(body["states"].as_array().map(Vec::len), Some(5));

    // 读不到设备：200 + unknown（不是 missing），服务端资产缺失时摘要/字节数为 null。
    let body = oww_state_payload("d1", None, None, None, 7);
    assert_eq!(body["vad"]["state"], "unknown");
    assert_eq!(body["ort"]["state"], "unknown");
    assert!(body["vad"]["sha256"].is_null());
    assert!(body["vad"]["bytes"].is_null());
    assert!(body["ort"]["sha256"].is_null());
    assert!(body["free_mb"].is_null());
}
