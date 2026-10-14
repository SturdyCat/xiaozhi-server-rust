//! 模板解析：把 `把{room}的灯{action}` 形式拆为字面(Lit)与槽位(Slot)的交错 token 序列。

use crate::plugins::nlu::norm;

/// 模板 token：交替的字面锚点与槽位占位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    Lit(String),
    Slot(String),
}

/// 已解析模板：token 序列（字面与槽位交替，且无相邻 Slot）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub tokens: Vec<Tok>,
}

/// 把模板字符串解析为 token 序列；字面与槽位值都经 [`norm`] 归一化，槽位名保持原样（标识符）。
///
/// 例：`把{room}的灯{action}` → `[Lit("把"), Slot("room"), Lit("的灯"), Slot("action")]`。
pub fn parse_template(s: &str) -> Template {
    let mut tokens = Vec::new();
    let mut lit = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            // 收尾当前字面（若有）
            if !lit.is_empty() {
                tokens.push(Tok::Lit(norm(&lit)));
                lit.clear();
            }
            // 收集到 '}'
            let mut name = String::new();
            for cc in chars.by_ref() {
                if cc == '}' {
                    break;
                }
                name.push(cc);
            }
            if !name.is_empty() {
                tokens.push(Tok::Slot(name));
            }
        } else {
            lit.push(c);
        }
    }
    if !lit.is_empty() {
        tokens.push(Tok::Lit(norm(&lit)));
    }
    Template { tokens }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lit_slot_lit_slot() {
        let t = parse_template("把{room}的灯{action}");
        assert_eq!(
            t.tokens,
            vec![
                Tok::Lit("把".into()),
                Tok::Slot("room".into()),
                Tok::Lit("的灯".into()),
                Tok::Slot("action".into()),
            ]
        );
    }

    #[test]
    fn normalizes_literals_and_strips_punctuation() {
        // 字面里的标点应被归一化掉，槽位名不受影响
        let t = parse_template("打开{room}的灯！");
        assert_eq!(
            t.tokens,
            vec![
                Tok::Lit("打开".into()),
                Tok::Slot("room".into()),
                Tok::Lit("的灯".into()),
            ]
        );
    }

    #[test]
    fn trailing_slot_without_literal() {
        let t = parse_template("{content}");
        assert_eq!(t.tokens, vec![Tok::Slot("content".into())]);
    }
}
