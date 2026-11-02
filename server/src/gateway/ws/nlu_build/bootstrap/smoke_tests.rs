//! 冒烟解析与门禁判定的纯逻辑回归。
//!
//! 分文件是为了守住 `.rs` 600 行的约定；判定规则本身仍由 `smoke.rs` 的
//! `parse_smoke` / `gate_verdict` 导出，这里只锁"什么该失败、失败码叫什么"。

use super::*;

fn labels() -> Vec<String> {
    vec!["reminder_countdown".into(), "other".into()]
}

fn smoke_line(positives: usize, negatives: usize, truncated: bool) -> String {
    let mut cases: Vec<String> = (0..positives)
        .map(|i| format!(r#"{{"text":"提醒我第{i}件事","expected":"reminder_countdown"}}"#))
        .collect();
    for i in 0..negatives {
        cases.push(format!(
            r#"{{"text":"今天天气怎么样{i}","expected":"other"}}"#
        ));
    }
    format!(
        "[train] done → /work/output/model.onnx\n[train] smoke={{\"cases\":[{}],\"truncated\":{truncated}}}",
        cases.join(",")
    )
}

#[test]
fn smoke_line_must_be_complete_consistent_and_untruncated() {
    let plan = parse_smoke(&smoke_line(1, 3, false), &labels()).unwrap();
    assert_eq!(plan.cases.len(), 4);
    assert!(!plan.truncated);
    assert_eq!(plan.cases[0].expected, "reminder_countdown");

    // 取最后一条 smoke 行（重复打印时以最新为准）。
    let twice = format!(
        "{}\n[train] smoke={{\"cases\":[{{\"text\":\"提醒我\",\"expected\":\"reminder_countdown\"}},\
         {{\"text\":\"天气1\",\"expected\":\"other\"}},{{\"text\":\"天气2\",\"expected\":\"other\"}},\
         {{\"text\":\"天气3\",\"expected\":\"other\"}}],\"truncated\":false}}",
        smoke_line(1, 3, false)
    );
    assert_eq!(parse_smoke(&twice, &labels()).unwrap().cases.len(), 4);

    let missing = "[train] done → /work/output/model.onnx\n";
    assert_eq!(
        parse_smoke(missing, &labels()).unwrap_err().kind,
        "smoke_missing"
    );
    // 模板没展开 = 喂训练时没见过的花括号句，必须拒绝而不是当正例判失败。
    let braces = r#"[train] smoke={"cases":[{"text":"提醒我{task}","expected":"reminder_countdown"},{"text":"天1","expected":"other"},{"text":"天2","expected":"other"},{"text":"天3","expected":"other"}]}"#;
    assert_eq!(
        parse_smoke(braces, &labels()).unwrap_err().kind,
        "smoke_missing"
    );
    // 期望标签不在候选 labels 里 = 脚本与服务端目录不同源。
    let unknown = r#"[train] smoke={"cases":[{"text":"提醒我","expected":"not_a_skill"},{"text":"天1","expected":"other"},{"text":"天2","expected":"other"},{"text":"天3","expected":"other"}]}"#;
    assert_eq!(
        parse_smoke(unknown, &labels()).unwrap_err().kind,
        "smoke_missing"
    );
    // 负例不足 3 条是**数据缺失**，绝不能报成 smoke_negative（那是"理由假的"）。
    assert_eq!(
        parse_smoke(&smoke_line(1, 2, false), &labels())
            .unwrap_err()
            .kind,
        "smoke_missing"
    );
    // 正例 0 条同理：0/0 不是 100%。
    assert_eq!(
        parse_smoke(&smoke_line(0, 3, false), &labels())
            .unwrap_err()
            .kind,
        "smoke_missing"
    );
    // 同一意图重复出现会放大命中率，等于换了判定口径。
    let dup = r#"[train] smoke={"cases":[{"text":"提醒我1","expected":"reminder_countdown"},{"text":"提醒我2","expected":"reminder_countdown"},{"text":"天1","expected":"other"},{"text":"天2","expected":"other"},{"text":"天3","expected":"other"}]}"#;
    assert_eq!(
        parse_smoke(dup, &labels()).unwrap_err().kind,
        "smoke_missing"
    );
    // 裁剪过 = 无法证明"负例 3/3"，单独一个失败码。
    assert_eq!(
        parse_smoke(&smoke_line(1, 3, true), &labels())
            .unwrap_err()
            .kind,
        "smoke_truncated"
    );
    assert_eq!(
        parse_smoke("[train] smoke=[]", &labels()).unwrap_err().kind,
        "smoke_missing"
    );
}

fn report(positives: usize, ok: usize) -> GateReport {
    GateReport {
        positives,
        ok,
        bad_class: 0,
        negatives: 3,
        rejected: 3,
        rejected_other: 3,
        rejected_threshold: 0,
        bad_class_detail: Vec::new(),
        rejected_detail: Vec::new(),
        elapsed_ms: 1_000,
    }
}

#[test]
fn gate_verdict_puts_bad_class_before_hit_rate_and_demands_three_negatives() {
    assert!(gate_verdict(&report(10, 9)).is_ok());
    assert!(gate_verdict(&report(4, 4)).is_ok());

    // 命中率不到 90%（4/4 → 3/4 只有 75%，不许通过）。
    assert_eq!(
        gate_verdict(&report(4, 3)).unwrap_err().kind,
        "smoke_positive"
    );
    assert_eq!(
        gate_verdict(&report(10, 8)).unwrap_err().kind,
        "smoke_positive"
    );

    // 危险失败零容忍：**即使命中率够**也必须先被拦下。
    let mut dangerous = report(10, 9);
    dangerous.bad_class = 1;
    assert_eq!(
        gate_verdict(&dangerous).unwrap_err().kind,
        "smoke_bad_class"
    );
    let mut perfect_but_dangerous = report(9, 9);
    perfect_but_dangerous.bad_class = 1;
    assert_eq!(
        gate_verdict(&perfect_but_dangerous).unwrap_err().kind,
        "smoke_bad_class"
    );

    // 负例必须 3/3。
    let mut leaky = report(4, 4);
    leaky.rejected = 2;
    assert_eq!(gate_verdict(&leaky).unwrap_err().kind, "smoke_negative");
    // 负例条数不足是**数据缺失**，必须是 smoke_missing，不能叫人去查模型。
    let mut short = report(4, 4);
    short.negatives = 2;
    short.rejected = 2;
    assert_eq!(gate_verdict(&short).unwrap_err().kind, "smoke_missing");

    // 超预算优先于命中率：超时说明机器有问题，不是模型不准。
    let mut slow = report(10, 10);
    slow.elapsed_ms = SMOKE_BUDGET_MS + 1;
    assert_eq!(gate_verdict(&slow).unwrap_err().kind, "smoke_timeout");

    // 正例 0 条是数据缺失，不是"0/0 = 100%"。
    assert_eq!(
        gate_verdict(&report(0, 0)).unwrap_err().kind,
        "smoke_missing"
    );
}
