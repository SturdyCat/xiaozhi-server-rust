//! `plugins/health.rs` 的单元测试。
//!
//! 全部走**临时目录**构造用例（不依赖宿主机上是否真的有 /data/models），
//! 无 sherpa feature 下同样可跑——探测本身只看文件系统，不构建引擎。

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

/// 轻量临时目录（零新增依赖：`std::env::temp_dir` + 唯一后缀）。
/// `Drop` 时递归清理，测试结束不留垃圾。
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("xiaozhi-health-{tag}-{ms}-{n}"));
        fs::create_dir_all(&p).expect("创建临时目录");
        Self(p)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 探测辅助：不存在的路径（各测试用不同后缀，避免并行互踩）。
fn ghost(suffix: &str) -> String {
    format!("/tmp/__xiaozhi_health_ghost_{suffix}__/no-such-file")
}

#[test]
fn asr_model_missing_fails_with_fix_hint() {
    let mut cfg = Config::default();
    cfg.asr.model = ghost("asr");
    let r = probe(&cfg, Capability::Asr).expect("asr 应有探测项");
    assert_eq!(r.state, ProbeState::Fail);
    let detail = r.detail.expect("失败必须带原因");
    assert!(detail.contains("asr].model"), "应点名字段: {detail}");
    assert!(detail.contains("不存在"), "{detail}");
    assert!(detail.contains("请"), "应含修复指引: {detail}");
}

#[test]
fn asr_model_present_ok() {
    let mut cfg = Config::default();
    let td = TempDir::new("asrok");
    let dir = td.path();
    let f = dir.join("model.int8.onnx");
    fs::write(&f, b"fake").unwrap();
    cfg.asr.model = f.to_string_lossy().into_owned();
    let r = probe(&cfg, Capability::Asr).expect("asr 应有探测项");
    assert_eq!(r.state, ProbeState::Ok);
    assert!(r.detail.is_none());
}

#[test]
fn vad_model_missing_fails() {
    let mut cfg = Config::default();
    cfg.vad.model = ghost("vad");
    let r = probe(&cfg, Capability::Vad).expect("vad 应有探测项");
    assert_eq!(r.state, ProbeState::Fail);
    assert!(r.detail.unwrap().contains("vad].model"));
}

/// 空路径是配置级问题（validate 拦），探测放行——不重复报错。
#[test]
fn empty_path_is_left_to_validate_not_probe() {
    let mut cfg = Config::default();
    cfg.asr.model = String::new();
    let r = probe(&cfg, Capability::Asr).unwrap();
    assert_eq!(r.state, ProbeState::Ok);
}

/// TTS：kokoro 选中时四路径缺一即 Fail，文案含「切到在线服务商」的出路。
#[test]
fn tts_kokoro_missing_paths_fail() {
    let mut cfg = Config::default();
    cfg.tts.engine = "kokoro".into();
    cfg.tts.kokoro.model = ghost("tts");
    cfg.tts.kokoro.voices = ghost("tts");
    cfg.tts.kokoro.tokens = ghost("tts");
    cfg.tts.kokoro.data_dir = ghost("tts");
    let r = probe(&cfg, Capability::Tts).unwrap();
    assert_eq!(r.state, ProbeState::Fail);
    let d = r.detail.unwrap();
    assert!(d.contains("tts.kokoro"), "{d}");
    assert!(d.contains("model"), "{d}");
    assert!(d.contains("xfyun"), "应给出在线替代方案: {d}");
}

/// TTS：kokoro 四路径齐全 → Ok。
#[test]
fn tts_kokoro_all_paths_ok() {
    let td = TempDir::new("ttsok");
    let dir = td.path();
    for name in ["model.onnx", "voices.bin", "tokens.txt"] {
        fs::write(dir.join(name), b"x").unwrap();
    }
    fs::create_dir_all(dir.join("espeak-ng-data")).unwrap();
    let mut cfg = Config::default();
    cfg.tts.engine = "kokoro".into();
    cfg.tts.kokoro.model = dir.join("model.onnx").to_string_lossy().into_owned();
    cfg.tts.kokoro.voices = dir.join("voices.bin").to_string_lossy().into_owned();
    cfg.tts.kokoro.tokens = dir.join("tokens.txt").to_string_lossy().into_owned();
    cfg.tts.kokoro.data_dir = dir.join("espeak-ng-data").to_string_lossy().into_owned();
    let r = probe(&cfg, Capability::Tts).unwrap();
    assert_eq!(r.state, ProbeState::Ok);
}

/// TTS：切到 xfyun（远程服务）后无本地探测——凭据/上游连通属配置级。
#[test]
fn tts_remote_engine_has_no_local_probe() {
    let mut cfg = Config::default();
    cfg.tts.engine = "xfyun".into();
    let r = probe(&cfg, Capability::Tts).unwrap();
    assert_eq!(r.state, ProbeState::Ok);
}

/// 记忆：父目录不存在 → Fail（含"请先创建目录"）。
#[test]
fn memory_missing_dir_fails() {
    let mut cfg = Config::default();
    cfg.memory.db_path = format!("{}/mem.db", ghost("memdir"));
    let r = probe(&cfg, Capability::Memory).unwrap();
    assert_eq!(r.state, ProbeState::Fail);
    let d = r.detail.unwrap();
    assert!(d.contains("memory].db_path"), "应点名字段: {d}");
    assert!(d.contains("创建目录"), "{d}");
}

/// 记忆：`:memory:` 直接放行（内存库无父目录可查）。
#[test]
fn memory_in_memory_db_ok() {
    let mut cfg = Config::default();
    cfg.memory.db_path = ":memory:".into();
    let r = probe(&cfg, Capability::Memory).unwrap();
    assert_eq!(r.state, ProbeState::Ok);
}

/// 记忆：可写目录（含 SQLite 库文件已存在的情形）→ Ok，且**探测后目录里不残留
/// `.health-probe`**（幂等无累积副作用，PRD 验收 6）。
#[test]
fn memory_writable_dir_ok_and_leaves_no_trace() {
    let td = TempDir::new("memok");
    let dir = td.path();
    let mut cfg = Config::default();
    cfg.memory.db_path = dir.join("memory.db").to_string_lossy().into_owned();
    // 连调两次：幂等性验证（第二次结果一致 + 无残留文件累积）
    for _ in 0..2 {
        let r = probe(&cfg, Capability::Memory).unwrap();
        assert_eq!(r.state, ProbeState::Ok);
    }
    let leftover: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".health-probe"))
        .collect();
    assert!(leftover.is_empty(), "探测不得残留临时文件: {leftover:?}");
}

/// 其余 7 个能力无本地探测项（网络/凭据类属配置级，被验收 6 禁止探测）。
#[test]
fn non_fs_capabilities_have_no_probe() {
    let cfg = Config::default();
    for cap in [
        Capability::Llm,
        Capability::FullChain,
        Capability::Voices,
        Capability::Firmware,
        Capability::Soul,
        Capability::Command,
        Capability::Wake,
    ] {
        assert!(
            probe(&cfg, cap).is_none(),
            "{} 不应有本地探测项",
            cap.id()
        );
    }
}

/// probe 的 JSON 投影键契约（`/api/health/engines` 的 `probe` 字段）。
#[test]
fn probe_report_json_shape() {
    let ok = ProbeReport::ok();
    assert_eq!(ok.to_json()["state"], "ok");
    assert!(ok.to_json()["detail"].is_null());
    let fail = ProbeReport::fail("修一下".into());
    assert_eq!(fail.to_json()["state"], "fail");
    assert_eq!(fail.to_json()["detail"], "修一下");
}

/// 全部 11 个能力都已被 match 覆盖（穷尽性由编译器保证，这里再显式数一遍，
/// 防止"新增能力后 probe 忘了写 match 分支"以外的方式引入漂移——其实编译不过，
/// 这条是文档级断言）。
#[test]
fn every_capability_is_matched() {
    let cfg = Config::default();
    assert_eq!(crate::plugins::registry::ALL_CAPABILITIES.len(), 11);
    for &cap in crate::plugins::registry::ALL_CAPABILITIES {
        let _ = probe(&cfg, cap); // 只要不 panic 即覆盖
    }
}
