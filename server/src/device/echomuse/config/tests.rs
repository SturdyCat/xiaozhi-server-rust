//! [`super::config`]（`[echomuse]` 配置段 + 连接欢迎语决策）的回归测试。

use super::*;

#[test]
fn engine_默认与别名规范化() {
    let mut cfg = EchomuseConfig::default();
    assert_eq!(cfg.engine, "native");
    cfg.engine = "WASM".into();
    cfg.normalize();
    assert_eq!(cfg.engine, "wasm-policy");
    assert_eq!(cfg.engine_id(), "wasm-policy");

    cfg.engine = "future".into();
    cfg.normalize();
    assert_eq!(cfg.engine, "future", "未知引擎必须保留以便注册表显式报错");
}

#[test]
fn engine_旧配置键可读且normalize幂等() {
    let mut cfg: EchomuseConfig = toml::from_str("engine = \"wasm\"").unwrap();
    cfg.normalize();
    assert_eq!(cfg.engine, "wasm-policy");
    let once = cfg.engine.clone();
    cfg.normalize();
    assert_eq!(cfg.engine, once);
}

#[test]
fn welcome_默认含使用提示且normalize后仍有效() {
    let mut cfg = EchomuseConfig::default();
    cfg.normalize();
    assert!(
        !cfg.welcome.is_empty(),
        "默认欢迎语不能为空（默认值必须可开箱即用）"
    );
    let once = cfg.welcome.clone();
    cfg.normalize();
    assert_eq!(cfg.welcome, once, "normalize 必须幂等");
}

#[test]
fn connect_welcome_仅echomuse承载且文本非空() {
    let mut cfg = EchomuseConfig::default();
    // EchoMuse 承载 + 默认文案 → Some
    let w = connect_welcome("echomuse", &cfg).expect("默认应有欢迎语");
    assert_eq!(w, cfg.welcome.trim());
    // xiaozhi WS 承载（client_id 任意其他值）→ 恒 None，语义不外溢
    assert!(connect_welcome("web", &cfg).is_none());
    assert!(connect_welcome("", &cfg).is_none());
    // 文案留空 = 显式关闭（trim 后空白同义）
    cfg.welcome = "   ".to_string();
    assert!(
        connect_welcome("echomuse", &cfg).is_none(),
        "留空必须等于关闭"
    );
}
