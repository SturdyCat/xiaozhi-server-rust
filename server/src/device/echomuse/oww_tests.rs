//! OWW VAD 资产契约：文件来自模型卷（启动时下载），下发前必须已 typed 重写，
//! 缺失/畸形要给出可定位的错误。传输本身在 `shell_tests.rs` 用 loopback 设备验证。
use super::*;

/// 单字节 key 的长度定界字段（本文件的合成模型字段号都 < 16）。
fn bytes_field(number: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![(number << 3) | 2, payload.len() as u8];
    out.extend_from_slice(payload);
    out
}

/// 最小 ModelProto：graph(7) → initializer(5) → TensorProto{ dims, data_type=FLOAT,
/// name, raw_data }。形状与真实 HF 模型一致（权重在 `raw_data` 里）。
fn raw_float_model() -> Vec<u8> {
    let mut tensor = vec![0x08, 0x01, 0x10, 0x01];
    tensor.extend(bytes_field(8, b"w"));
    tensor.extend(bytes_field(9, &[0x00, 0x00, 0x80, 0x3F]));
    let graph = bytes_field(5, &tensor);
    bytes_field(7, &graph)
}

fn temp_models_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("oww-vad-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("oww")).expect("mkdir");
    dir
}

/// `XIAOZHI_MODELS_DIR` 是全局环境变量，测试必须串行（与其它 echomuse 测试同一把锁）。
struct Volume {
    dir: PathBuf,
}

impl Volume {
    fn new(model: Option<&[u8]>) -> Self {
        let dir = temp_models_dir();
        if let Some(model) = model {
            std::fs::write(dir.join(VAD_RELATIVE), model).expect("写模型");
        }
        std::env::set_var(ENV_MODELS_DIR, &dir);
        std::env::remove_var(ENV_OVERRIDE);
        Self { dir }
    }
}

impl Drop for Volume {
    fn drop(&mut self) {
        std::env::remove_var(ENV_MODELS_DIR);
        std::env::remove_var(ENV_OVERRIDE);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn vad_path_is_under_the_models_volume() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let volume = Volume::new(None);
    assert_eq!(vad_path(), volume.dir.join("oww/silero_vad.onnx"));
    // 刻意与 sherpa 的 silero_vad.onnx 分开：那份带 raw_data，装到设备会 SIGBUS。
    assert_ne!(vad_path(), volume.dir.join("silero_vad.onnx"));
}

#[test]
fn raw_model_is_converted_before_it_is_sent() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let _volume = Volume::new(Some(&raw_float_model()));

    let asset = vad_asset().expect("模型卷里的原始模型应可读");
    // raw_data 的 tag(0x4A)+长度(0x04) 必须消失，packed float_data 的 tag(0x22) 必须出现。
    assert!(
        !asset.windows(2).any(|window| window == [0x4A, 0x04]),
        "raw_data 未搬走：设备上的 ORT 会在 CreateSession SIGBUS"
    );
    assert!(
        asset
            .windows(6)
            .any(|window| window == [0x22, 0x04, 0x00, 0x00, 0x80, 0x3F]),
        "缺少 packed float_data"
    );
}

#[test]
fn already_typed_model_passes_through_unchanged() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let typed = onnx_typed::rewrite(&raw_float_model())
        .expect("rewrite")
        .bytes;
    let _volume = Volume::new(Some(&typed));
    assert_eq!(vad_asset().expect("typed 模型"), typed);
}

#[test]
fn missing_model_names_the_path_and_the_download_switch() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let volume = Volume::new(None);
    let error = vad_asset().expect_err("缺文件必须报错");
    assert!(
        error.contains(&volume.dir.join(VAD_RELATIVE).display().to_string()),
        "error was: {error}"
    );
    assert!(error.contains("OWW_VAD_URL"), "error was: {error}");
    assert!(error.contains(ENV_OVERRIDE), "error was: {error}");
}

#[test]
fn env_override_replaces_the_model_volume() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let _volume = Volume::new(Some(&raw_float_model()));
    let path = std::env::temp_dir().join(format!("oww-vad-{}.onnx", uuid::Uuid::new_v4()));
    std::fs::write(&path, raw_float_model()).unwrap();
    std::env::set_var(ENV_OVERRIDE, &path);
    assert_eq!(vad_path(), path);
    assert!(vad_asset().is_ok());
    let _ = std::fs::remove_file(&path);
}

/// 缓存按文件指纹失效：同一路径换了内容，`vad_asset()` 必须重新解析而不是吐旧字节。
///
/// 缓存是为高频只读调用（管理台轮询 `GET …/oww/state`、每次设备注册）加的优化，
/// 语义必须与"每次都读盘重转"完全一致——否则换模型后会一直下发旧资产。
#[test]
fn asset_cache_invalidates_when_the_file_changes() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let _volume = Volume::new(Some(&raw_float_model()));
    let path = std::env::temp_dir().join(format!("oww-vad-cache-{}.onnx", uuid::Uuid::new_v4()));
    std::fs::write(&path, raw_float_model()).unwrap();
    std::env::set_var(ENV_OVERRIDE, &path);

    let converted = vad_asset().expect("首次读取应成功转换");
    assert!(
        converted
            .windows(6)
            .any(|window| window == [0x22, 0x04, 0x00, 0x00, 0x80, 0x3F]),
        "首次读取应得到 packed float_data"
    );

    // 同路径换成畸形内容（长度也不同）：缓存必须失效，从内容重新判定为错误。
    std::fs::write(&path, b"<!DOCTYPE html><html>404</html>").unwrap();
    let error = vad_asset().expect_err("换内容后必须重新解析（缓存未失效就会吐旧字节）");
    assert!(error.contains("不是可解析的 ONNX"), "error was: {error}");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn unreadable_override_names_the_path_instead_of_falling_back() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let _volume = Volume::new(Some(&raw_float_model()));
    let missing = std::env::temp_dir().join("oww-vad-does-not-exist.onnx");
    std::env::set_var(ENV_OVERRIDE, &missing);
    let error = vad_asset().expect_err("覆盖路径不存在时不能悄悄回落到模型卷");
    assert!(
        error.contains("oww-vad-does-not-exist.onnx"),
        "error was: {error}"
    );
}

/// 空值等于"没配"：容器里 `-e XIAOZHI_OWW_VAD_PATH=` 是常见写法，不该当错误。
#[test]
fn blank_override_falls_back_to_the_model_volume() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let volume = Volume::new(Some(&raw_float_model()));
    std::env::set_var(ENV_OVERRIDE, "  ");
    assert_eq!(vad_path(), volume.dir.join(VAD_RELATIVE));
    assert!(vad_asset().is_ok());
}

/// 下载到 HTML 错误页 / 半截文件时，宁可 500 也不能把垃圾发给设备。
#[test]
fn malformed_model_is_rejected_with_its_path() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let volume = Volume::new(Some(b"<!DOCTYPE html><html>404</html>"));
    let error = vad_asset().expect_err("非 protobuf 必须报错");
    assert!(
        error.contains(&volume.dir.join(VAD_RELATIVE).display().to_string()),
        "error was: {error}"
    );
}

/// 来源必须与上游 `controller/Dockerfile` 的 `silero` 阶段一致：openWakeWord v0.5.1
/// 的 release 资产（不是随便某个镜像站），且两段摘要都按官方 pin。
#[test]
fn official_source_is_the_openwakeword_release_asset() {
    assert!(
        OFFICIAL_VAD_URL.contains("dscripka/openWakeWord/releases/download/v0.5.1/silero_vad.onnx"),
        "官方 Dockerfile 就是从该 URL 取的：{OFFICIAL_VAD_URL}"
    );
    for digest in [OFFICIAL_VAD_SHA256, TYPED_VAD_SHA256] {
        assert_eq!(digest.len(), 64, "sha256 必须是 64 位十六进制：{digest}");
        assert!(
            digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "sha256 必须是十六进制：{digest}"
        );
    }
    // 输入/输出是两份不同的字节（原地重写会改变序列化布局），不能写成同一个值。
    assert_ne!(OFFICIAL_VAD_SHA256, TYPED_VAD_SHA256);
}

#[test]
fn classify_recognizes_both_official_digests() {
    assert_eq!(classify(TYPED_VAD_SHA256), VadSource::Typed);
    assert_eq!(classify(OFFICIAL_VAD_SHA256), VadSource::Raw);
    // 大小写不敏感：摘要比对不该因为上游大小写差异而误判成"未验证"。
    assert_eq!(
        classify(&TYPED_VAD_SHA256.to_ascii_uppercase()),
        VadSource::Typed
    );
    assert_eq!(classify(&"0".repeat(64)), VadSource::Unverified);
}

/// 0 字节文件是"下载中断的残骸"，必须当成缺失去补下，而不是当成已就绪。
#[test]
fn empty_file_is_not_ready() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let volume = Volume::new(Some(b""));
    let path = volume.dir.join(VAD_RELATIVE);
    assert!(!is_ready(&path));
    assert!(!is_ready(&volume.dir.join("oww/absent.onnx")));
}

#[test]
fn write_atomic_leaves_no_part_file_and_refuses_to_clobber_on_rename() {
    let dir = temp_models_dir();
    let target = dir.join("oww/silero_vad.onnx");
    write_atomic(&target, b"first").expect("写入");
    assert_eq!(std::fs::read(&target).unwrap(), b"first");
    write_atomic(&target, b"second").expect("覆盖");
    assert_eq!(std::fs::read(&target).unwrap(), b"second");
    // `.part` 是中间态，成功后不能留在模型卷里（否则下次有人误当模型）。
    assert!(!target.with_extension("onnx.part").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 已就绪时 `ensure_vad_model` 直接返回，**不发任何网络请求**（离线部署必须能启动）。
#[tokio::test]
async fn ensure_returns_ready_file_without_network() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let model = raw_float_model();
    let volume = Volume::new(Some(&model));
    let path = ensure_vad_model(false).await.expect("已就绪不该失败");
    assert_eq!(path, volume.dir.join(VAD_RELATIVE));
    // 读时转换不该改写磁盘：原始 raw_data 版依然原样躺在模型卷里。
    assert_eq!(std::fs::read(&path).unwrap(), model);
}

/// 用户显式指了 `XIAOZHI_OWW_VAD_PATH` 时**不自动下载**（不往用户指定位置写东西）。
#[tokio::test]
async fn prefetch_is_skipped_when_override_is_set() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let volume = Volume::new(None);
    let custom = volume.dir.join("custom/my-vad.onnx");
    std::env::set_var(ENV_OVERRIDE, &custom);
    spawn_prefetch();
    // 给后台任务一个让出机会：若真的 spawn 了下载，会先建目录/写 .part。
    tokio::task::yield_now().await;
    assert!(!custom.exists(), "覆盖路径不该被自动写入");
    std::env::remove_var(ENV_OVERRIDE);
}

/// 「只有开启 EchoMuse 才下载」是这次改动的核心约束：入口脚本原先无条件补它，
/// 现在必须由 main 的 `[echomuse].enabled` 分支触发。
#[test]
fn prefetch_is_wired_under_the_echomuse_switch_only() {
    let main_rs = include_str!("../../main.rs");
    let enabled_at = main_rs
        .find("if config.echomuse.enabled")
        .expect("main 必须按 [echomuse].enabled 分支启动 EchoMuse");
    let call_at = main_rs
        .find("oww::spawn_prefetch()")
        .expect("main 必须触发设备语音门模型预取");
    assert!(
        call_at > enabled_at,
        "spawn_prefetch 必须在 [echomuse].enabled 分支内，未启用时不得下载"
    );

    let entrypoint = include_str!("../../../../docker-entrypoint.sh");
    assert!(
        !entrypoint.contains("ensure_oww_vad") && !entrypoint.contains("OWW_VAD_URL"),
        "入口脚本不得再无条件下载设备语音门模型（未启用 EchoMuse 的部署不该多下字节）"
    );
}

/// 真实下载官方资产：**输入**摘要对 [`OFFICIAL_VAD_SHA256`]、重写后**输出**摘要对
/// [`TYPED_VAD_SHA256`]。这是「来源没跑偏 + 重写器与官方 silero_typed.py 等价」的
/// 端到端凭据，需要网络，故默认 ignored（手动 `--ignored` 跑）。
#[tokio::test]
#[ignore = "需要网络：验证官方来源与 typed 重写都和上游钉的摘要一致"]
async fn live_download_matches_the_official_digests() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let volume = Volume::new(None);
    let path = ensure_vad_model(false).await.expect("下载官方资产应成功");
    assert_eq!(path, volume.dir.join(VAD_RELATIVE));
    let bytes = std::fs::read(&path).expect("读回下载结果");
    assert_eq!(
        sha256_hex(&bytes),
        OFFICIAL_VAD_SHA256,
        "下载到的应是官方输入那份（openWakeWord v0.5.1）"
    );
    let typed = vad_asset().expect("重写应成功");
    assert_eq!(
        sha256_hex(&typed),
        TYPED_VAD_SHA256,
        "重写结果必须与官方 silero_typed.py 的输出摘要一致"
    );
}

/// 部署环境（compose 默认）常靠 `GITHUB_PROXY` 才能访问 GitHub：服务端这条下载路
/// 必须与入口脚本的 `apply_proxy` 同语义——只对 github.com 套前缀，`off` 表示直连。
#[test]
fn github_proxy_applies_only_to_github_urls() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    std::env::remove_var(ENV_GITHUB_PROXY);
    assert_eq!(with_github_proxy(OFFICIAL_VAD_URL), OFFICIAL_VAD_URL);

    std::env::set_var(ENV_GITHUB_PROXY, "https://tvv.tw/");
    assert_eq!(
        with_github_proxy(OFFICIAL_VAD_URL),
        format!("https://tvv.tw/{OFFICIAL_VAD_URL}")
    );
    // 内网镜像不是 github.com，不能被二次套前缀。
    let mirror = "https://mirror.internal/oww/silero_vad.onnx";
    assert_eq!(with_github_proxy(mirror), mirror);

    std::env::set_var(ENV_GITHUB_PROXY, "off");
    assert_eq!(with_github_proxy(OFFICIAL_VAD_URL), OFFICIAL_VAD_URL);
    std::env::remove_var(ENV_GITHUB_PROXY);
}

/// 与入口脚本共用的总开关：`off` 不得下载，`force` 忽略已有文件，未设/未知按 `missing`。
#[test]
fn auto_download_mode_matches_the_entrypoint_semantics() {
    assert_eq!(auto_download(None), AutoDownload::Missing);
    assert_eq!(auto_download(Some("")), AutoDownload::Missing);
    assert_eq!(auto_download(Some("  missing ")), AutoDownload::Missing);
    // 拼错的取值按默认处理（宁可补一次，也别静默缺失）。
    assert_eq!(auto_download(Some("of")), AutoDownload::Missing);
    assert_eq!(auto_download(Some("OFF")), AutoDownload::Off);
    assert_eq!(auto_download(Some("Force")), AutoDownload::Force);
}

/// `XIAOZHI_AUTO_DOWNLOAD_MODELS=off` 时连后台任务都不该起（部署方明确不联网下模型）。
#[tokio::test]
async fn prefetch_respects_the_auto_download_off_switch() {
    let _guard = super::super::hub_tests::HUB_LOCK.lock().unwrap();
    let volume = Volume::new(None);
    std::env::set_var(ENV_AUTO_DOWNLOAD, "off");
    spawn_prefetch();
    tokio::task::yield_now().await;
    assert!(
        !volume.dir.join(VAD_RELATIVE).exists(),
        "off 时不得创建/下载模型文件"
    );
    // 0 字节残骸也不该被补下：这条同时说明"off 优先于缺失判断"。
    std::env::remove_var(ENV_AUTO_DOWNLOAD);
}
