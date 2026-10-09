//! `server/src/engine.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;

fn sig_lock(s: &str) -> RwLock<String> {
    RwLock::new(s.to_string())
}

/// P2 验收（正面）：改 `[llm].model` → 决策为"需要重建"，且新旧签名不同
///（新签名即新会话将使用的引擎）。
#[test]
fn llm_model_change_triggers_rebuild() {
    let mut fresh = Config::default();
    fresh.llm.api_key = "sk-test".into();
    fresh.llm.model = "gpt-4o".into();
    let old = capability_signature(&fresh, Capability::Llm);
    let lock = sig_lock(&old);

    assert!(
        decide_refresh(Capability::Llm, &lock, &fresh)
            .unwrap()
            .is_none(),
        "配置未变 → 不应重建（会话开始调用必须零开销）"
    );

    fresh.llm.model = "gpt-4o-mini".into();
    let decision = decide_refresh(Capability::Llm, &lock, &fresh).unwrap();
    let new_sig = decision.expect("改 model 应触发重建");
    assert_ne!(new_sig, old, "签名应随 model 变化");
}

/// P2 验收（反面）：坏配置**不替换**正在工作的引擎，并给出可行动原因。
#[test]
fn llm_bad_config_refuses_to_replace_engine() {
    let mut healthy = Config::default();
    healthy.llm.api_key = "sk-test".into();
    let lock = sig_lock(&capability_signature(&healthy, Capability::Llm));

    // 1) 密钥被清空（P3 的"空 = 保留原值"约定下这属于坏配置，不能拿它覆盖旧引擎）
    let mut wiped = healthy.clone();
    wiped.llm.api_key = String::new();
    let err = decide_refresh(Capability::Llm, &lock, &wiped).expect_err("空密钥应拒绝替换");
    assert!(err.contains("api_key"), "原因应点名 api_key: {err}");

    // 2) 接口地址被清空 → 硬校验失败
    let mut no_base = healthy.clone();
    no_base.llm.api_base = String::new();
    let err = decide_refresh(Capability::Llm, &lock, &no_base).expect_err("空 api_base 应拒绝替换");
    assert!(err.contains("api_base"), "原因应点名 api_base: {err}");
}

/// TTS：签名变化触发重建；凭据缺失则拒绝（保留旧引擎）。
///
/// ⚠️ 主断言走 **xfyun**（在线实现，无需 `sherpa` feature）；本地 Kokoro 分支按
/// feature 分别断言——无 feature 时"拒绝替换"才是正确行为（而不是静默失败）。
#[test]
fn tts_refresh_decisions() {
    let mut xfyun_ok = Config::default();
    xfyun_ok.tts.engine = "xfyun".into();
    xfyun_ok.tts.xfyun.app_id = "app".into();
    xfyun_ok.tts.xfyun.api_key = "key".into();
    xfyun_ok.tts.xfyun.api_secret = "secret".into();
    xfyun_ok.tts.xfyun.voice = "xiaoyan".into();

    let lock = sig_lock(&capability_signature(&xfyun_ok, Capability::Tts));
    assert!(
        decide_refresh(Capability::Tts, &lock, &xfyun_ok)
            .unwrap()
            .is_none(),
        "同一份 xfyun 配置 → 无需重建"
    );

    let mut changed_voice = xfyun_ok.clone();
    changed_voice.tts.xfyun.voice = "x4_lingxiaoxuan".into();
    assert!(
        decide_refresh(Capability::Tts, &lock, &changed_voice)
            .unwrap()
            .is_some(),
        "改音色应触发重建"
    );

    let mut missing_secret = xfyun_ok.clone();
    missing_secret.tts.xfyun.api_secret = String::new();
    let err =
        decide_refresh(Capability::Tts, &lock, &missing_secret).expect_err("凭据不全应拒绝切引擎");
    assert!(err.contains("api_secret"), "原因应点名缺哪个凭据: {err}");

    // 本地 Kokoro：按 feature 断言
    let sherpa_cfg = Config::default();
    let sherpa_lock = sig_lock(&capability_signature(&sherpa_cfg, Capability::Tts));
    match decide_refresh(Capability::Tts, &sherpa_lock, &sherpa_cfg) {
        Ok(d) if cfg!(feature = "sherpa") => assert!(d.is_none(), "未改动的 sherpa 配置无需重建"),
        Err(e) if !cfg!(feature = "sherpa") => {
            assert!(e.contains("sherpa"), "无 feature 时应给出可行动原因: {e}")
        }
        other => panic!("与 feature 组合不符的决策结果: {other:?}"),
    }
}
