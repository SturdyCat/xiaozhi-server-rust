//! `server/src/app/voices.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;

/// 无硬编码可用列表的契约：内存态初始为空目录（仅探测/缓存填充）。
#[test]
fn state_starts_empty_without_cache() {
    std::env::set_var("XIAOZHI_VOICES_CACHE", "/nonexistent/voices-test.json");
    init();
    let snap = state().read().unwrap_or_else(|e| e.into_inner()).clone();
    assert!(snap.voices.is_empty(), "无缓存时应为空目录（不得回退硬编码可用列表）");
    assert_eq!(snap.source, "empty");
    std::env::remove_var("XIAOZHI_VOICES_CACHE");
}

/// 候选池契约：vcn 唯一、gender/type 合法（候选只是探测输入，不是可用列表）。
#[test]
fn candidates_are_unique_and_wellformed() {
    let c = candidates();
    let mut seen = std::collections::HashSet::new();
    for v in &c {
        assert!(seen.insert(v.vcn.clone()), "候选 vcn 重复: {}", v.vcn);
        assert!(matches!(v.gender.as_str(), "female" | "male"), "非法性别: {}", v.vcn);
        assert!(matches!(v.type_.as_str(), "classic" | "x6"), "非法类型: {}", v.vcn);
    }
    assert!(c.len() >= 100, "候选池过小（疑似被截断）: {}", c.len());
}

/// Voice 序列化：`type` 字段名（客户端按此解析）。
#[test]
fn voice_serializes_type_field() {
    let v = Voice {
        vcn: "xiaoyan".into(),
        name: "小燕".into(),
        gender: "female".into(),
        type_: "classic".into(),
        tag: "标准女声".into(),
    };
    let j = serde_json::to_value(&v).unwrap();
    assert_eq!(j["vcn"], "xiaoyan");
    assert_eq!(j["gender"], "female");
    assert_eq!(j["type"], "classic");
}

/// 真实探测冒烟（**排障工具，日常不跑**）：三要素经环境变量传入——
/// `XFYUN_APP_ID=… XFYUN_API_KEY=… XFYUN_API_SECRET=… cargo test voices_probe_live -- --ignored --nocapture`
#[test]
#[ignore = "live AIUI 调用：需 XFYUN_APP_ID/XFYUN_API_KEY/XFYUN_API_SECRET 环境变量"]
fn voices_probe_live() {
    let (Ok(app_id), Ok(api_key), Ok(api_secret)) = (
        std::env::var("XFYUN_APP_ID"),
        std::env::var("XFYUN_API_KEY"),
        std::env::var("XFYUN_API_SECRET"),
    ) else {
        eprintln!("跳过：未设置 XFYUN_APP_ID/XFYUN_API_KEY/XFYUN_API_SECRET");
        return;
    };
    let cfg = XfyunTtsConfig { app_id, api_key, api_secret, voice: String::new() };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let t0 = std::time::Instant::now();
    rt.block_on(probe_and_persist(cfg));
    let snap = state().read().unwrap_or_else(|e| e.into_inner()).clone();
    println!(
        "== 探测完成：{} 条可用（耗时 {}s），示例: {:?}",
        snap.voices.len(),
        t0.elapsed().as_secs(),
        snap.voices.first().map(|v| (&v.vcn, &v.name, &v.type_, v.gender.as_str()))
    );
}
