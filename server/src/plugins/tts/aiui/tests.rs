//! `server/src/plugins/tts/aiui.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;

/// pcm 转换：0 / 满幅 / 半幅 / 奇数字节防御。
#[test]
fn pcm_conversion_roundtrip() {
    let bytes = [0x00u8, 0x00, 0xFF, 0x7F, 0x00, 0x80, 0x00, 0x40];
    let v = pcm_i16le_to_f32(&bytes);
    assert_eq!(v.len(), 4);
    assert!((v[0]).abs() < 1e-6);
    assert!((v[1] - 32767.0 / 32768.0).abs() < 1e-6);
    assert!((v[2] + 1.0).abs() < 1e-6);
    assert!((v[3] - 0.5).abs() < 1e-6);
    assert_eq!(pcm_i16le_to_f32(&[1, 2, 3]).len(), 1);
}

/// 音色回退：配置为空 → xiaoyan（请求体与错误提示一致）。
#[test]
fn effective_voice_falls_back_to_xiaoyan() {
    let empty = XfyunTtsConfig { app_id: "a".into(), api_key: "k".into(), api_secret: "s".into(), voice: "  ".into() };
    assert_eq!(effective_voice(&empty), "xiaoyan");
    let set = XfyunTtsConfig { voice: " x4_lingxiaoqi_oral ".into(), ..empty };
    assert_eq!(effective_voice(&set), "x4_lingxiaoqi_oral");
}

/// 真实连 AIUI 的合成/探测冒烟（**排障工具，日常不跑**）：密钥经环境变量——
/// `XFYUN_APP_ID=… XFYUN_API_KEY=… XFYUN_API_SECRET=… [XFYUN_VOICE=…] \
///  cargo test aiui_tts_live -- --ignored --nocapture`
#[test]
#[ignore = "live AIUI 调用：需 XFYUN_APP_ID/XFYUN_API_KEY/XFYUN_API_SECRET 环境变量"]
fn aiui_tts_live() {
    let (Ok(app_id), Ok(api_key), Ok(api_secret)) = (
        std::env::var("XFYUN_APP_ID"),
        std::env::var("XFYUN_API_KEY"),
        std::env::var("XFYUN_API_SECRET"),
    ) else {
        eprintln!("跳过：未设置 XFYUN_APP_ID/XFYUN_API_KEY/XFYUN_API_SECRET");
        return;
    };
    let cfg = XfyunTtsConfig { app_id, api_key, api_secret, voice: "xiaoyan".into() };
    // 1) 整段合成
    let t0 = Instant::now();
    let tts = AiuiTts::new(&cfg).expect("构造引擎");
    let total = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = total.clone();
    let r = tts.synthesize_stream("你好，今天天气怎么样", 1.0, 0, Box::new(move |_sr, chunk| {
        counter.fetch_add(chunk.len(), std::sync::atomic::Ordering::Relaxed);
        true
    }));
    println!("== 合成: {:?}（{} 样本，{}ms）", r.map(|()| "OK".to_string()).map_err(|e| format!("{e:#}")), total.load(std::sync::atomic::Ordering::Relaxed), t0.elapsed().as_millis());
    // 2) 音色探测：普通 / 超拟人（v2/tts 不可用）
    for vcn in ["xiaoyan", "x4_lingxiaoqi_oral", "x6_dongmanshaonv_pro", "x2_xiaojuan"] {
        let t1 = Instant::now();
        println!("== 探测[{vcn}]: {:?}（{}ms）", probe_voice(&cfg, vcn).map(|b| format!("OK {b}B")).map_err(|e| format!("{e:#}")), t1.elapsed().as_millis());
    }
}
