//! [`super`]（`[wakeup]` 唤醒问候语）的回归测试。
//!
//! 两条主线：**默认开启即开箱可用**（产品决策：问候语是标准体验，不需要的用户显式关闭），
//! 以及**配置矛盾必须被拦下并给出修法**（空问候语、过长问候语）。后者是本仓库最忌讳的
//! 静默失败面。

use super::*;

#[test]
fn 默认开启且问候() {
    let c = WakeConfig::default();
    assert!(
        c.enabled,
        "默认必须开启（产品决策：唤醒问候是「开机即用」的标准体验；不需要的用户显式关闭）"
    );
    assert_eq!(c.greeting, "我在");
    assert!(c.persist, "默认允许落盘复用（启用后才有意义）");
    assert!(c.active(), "默认配置必须真的会问候");

    let g = WakeGreeting::new(&c);
    assert!(g.active());
    assert_eq!(g.text(), "我在", "单句池恒返回它");
}

#[test]
fn active_要求启用且问候语非空() {
    let mut c = WakeConfig {
        enabled: true,
        ..Default::default()
    };
    assert!(c.active());
    c.greeting = "   ".to_string();
    assert!(!c.active(), "空白问候语等同没填");
    c.enabled = false;
    c.greeting = "我在".to_string();
    assert!(!c.active(), "关闭时即使有文本也不问候");
}

#[test]
fn normalize_去空白且幂等() {
    let mut c = WakeConfig {
        greeting: "  我在  ".to_string(),
        ..Default::default()
    };
    c.normalize();
    assert_eq!(c.greeting, "我在");
    let once = c.greeting.clone();
    c.normalize();
    assert_eq!(c.greeting, once, "normalize 必须幂等");
}

#[test]
fn 启用但问候语为空_报错并给出修法() {
    let c = WakeConfig {
        enabled: true,
        greeting: String::new(),
        ..Default::default()
    };
    let e = c.validate().expect_err("必须拦下");
    let msg = format!("{e}");
    assert!(
        msg.contains("[wakeup].greeting"),
        "错误文案要指出字段：{msg}"
    );
    assert!(
        msg.contains("enabled"),
        "错误文案要给出修法（关掉或填上）：{msg}"
    );
}

#[test]
fn 关闭时空问候语不报错() {
    // 关闭态下这些字段根本不生效，不该拦住启动。
    let c = WakeConfig {
        enabled: false,
        greeting: String::new(),
        ..Default::default()
    };
    assert!(c.validate().is_ok());
}

#[test]
fn 问候语过长_报错并说明代价() {
    let c = WakeConfig {
        enabled: true,
        greeting: "我".repeat(GREETING_SOFT_LIMIT + 1),
        ..Default::default()
    };
    let e = c.validate().expect_err("必须拦下");
    let msg = format!("{e}");
    assert!(msg.contains("排队"), "要说明为什么不该太长：{msg}");
    // 恰好在上限上不报错
    let ok = WakeConfig {
        enabled: true,
        greeting: "我".repeat(GREETING_SOFT_LIMIT),
        ..Default::default()
    };
    assert!(ok.validate().is_ok());
}

#[test]
fn signature_覆盖全部行为项() {
    let base = WakeConfig::default().signature();
    for changed in [
        WakeConfig {
            enabled: false,
            ..Default::default()
        },
        WakeConfig {
            greeting: "在呢".to_string(),
            ..Default::default()
        },
        WakeConfig {
            persist: false,
            ..Default::default()
        },
    ] {
        assert_ne!(base, changed.signature(), "任一字段变化都必须换签名");
    }
    assert_eq!(base, WakeConfig::default().signature(), "同配置签名稳定");
}

#[test]
fn 会话快照与配置一致且去空白() {
    let c = WakeConfig {
        enabled: true,
        greeting: "  在呢  ".to_string(),
        persist: false,
        ..Default::default()
    };
    let g = WakeGreeting::new(&c);
    assert!(g.active());
    assert_eq!(g.text(), "在呢", "快照必须去空白（TTS 与缓存键都用它）");
    assert!(!g.persist());

    // 快照是**副本**：之后改配置不影响已构造的快照（NextSession 语义）
    let mut c2 = c.clone();
    c2.greeting = "换一句".to_string();
    assert_eq!(g.text(), "在呢");
}

#[test]
fn 多句问候_池拆分与normalize收敛() {
    let mut c = WakeConfig {
        greeting: " 我在 \n\n  在呢  \n\n你好呀\n".to_string(),
        ..Default::default()
    };
    assert_eq!(
        c.pool(),
        vec!["我在".to_string(), "在呢".to_string(), "你好呀".to_string()],
        "换行拆分、逐行去空白、丢空行"
    );
    c.normalize();
    assert_eq!(c.greeting, "我在\n在呢\n你好呀", "normalize 收敛为规范形态");
    let once = c.greeting.clone();
    c.normalize();
    assert_eq!(c.greeting, once, "多句 normalize 必须幂等");
}

#[test]
fn 全空白行_等同没填() {
    let c = WakeConfig {
        enabled: true,
        greeting: "  \n\n\t\n".to_string(),
        ..Default::default()
    };
    assert!(!c.active(), "纯空白行不能变成「问候一个空字符串」");
    assert!(c.validate().is_err(), "启用但池空必须拦下");
}

#[test]
fn 多句中任何一句过长_报错并指出该句() {
    let long = "哇".repeat(GREETING_SOFT_LIMIT + 1);
    let c = WakeConfig {
        enabled: true,
        greeting: format!("我在\n{long}"),
        ..Default::default()
    };
    let e = c.validate().expect_err("超限句必须拦下");
    assert!(
        format!("{e}").contains(&long),
        "文案要能定位到具体哪句：{e}"
    );
}

#[test]
fn 多句快照_随机选句且必属池() {
    let c = WakeConfig {
        enabled: true,
        greeting: "甲\n乙\n丙".to_string(),
        ..Default::default()
    };
    let g = WakeGreeting::new(&c);
    let pool = ["甲", "乙", "丙"];
    let mut seen = std::collections::HashSet::new();
    for _ in 0..100 {
        let t = g.text();
        assert!(pool.contains(&t), "随机结果必须来自池内: {t:?}");
        seen.insert(t.to_string());
    }
    assert!(seen.len() >= 2, "100 次取样只有 {seen:?} 一种，随机失效");
}
