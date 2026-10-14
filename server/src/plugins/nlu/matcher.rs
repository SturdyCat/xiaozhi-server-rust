//! 锚点匹配：在归一化文本中按顺序不重叠定位字面锚点，抽取锚点之间的子串作为槽位，并校验封闭槽位值。

use std::collections::HashMap;

use crate::plugins::nlu::template::{Tok, Template};

/// 在 `hay` 中查找 `needle` 子序列的起始 char 索引，无则返回 None。
fn find_subseq(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// 匹配模板：返回槽位名 → 抽取值的映射；任一锚点缺失或封闭槽位值不匹配则返回 None。
///
/// 确定性：字面锚点按 token 顺序在文本中从左到右不重叠定位；槽位抽取为相邻锚点之间的子串。
/// 不强制消费完整文本（语音识别可能多说"谢谢"之类后缀），只要求模板锚点全部命中。
pub fn match_template(
    text: &str,
    tpl: &Template,
    slot_values: &HashMap<String, Vec<String>>,
    slot_optional: &HashMap<String, bool>,
) -> Option<HashMap<String, String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut cur = 0usize; // 当前起始 char 索引
    let mut result: HashMap<String, String> = HashMap::new();

    let mut idx = 0;
    while idx < tpl.tokens.len() {
        match &tpl.tokens[idx] {
            Tok::Lit(s) => {
                let needle: Vec<char> = s.chars().collect();
                match find_subseq(&chars[cur..], &needle) {
                    Some(rel) => {
                        cur = cur + rel + needle.len();
                        idx += 1;
                    }
                    None => return None,
                }
            }
            Tok::Slot(name) => {
                // 槽位后的结束锚点 = 后续第一个可达字面（其 char 起始）；若无则到结尾。
                let mut end_lit: Option<usize> = None;
                for t in &tpl.tokens[idx + 1..] {
                    if let Tok::Lit(s) = t {
                        let needle: Vec<char> = s.chars().collect();
                        if let Some(rel) = find_subseq(&chars[cur..], &needle) {
                            end_lit = Some(cur + rel);
                            break;
                        }
                    }
                }
                match end_lit {
                    Some(end) => {
                        // 有结束锚点：槽位值 = 锚点之间的子串，按封闭值表校验。
                        let value: String = chars[cur..end].iter().collect();
                        if let Some(values) = slot_values.get(name) {
                            if !values.is_empty() {
                                let ok = values.iter().any(|v| v == &value)
                                    || (slot_optional.get(name).copied().unwrap_or(false)
                                        && value.is_empty());
                                if !ok {
                                    return None;
                                }
                            }
                        }
                        result.insert(name.clone(), value);
                        cur = end;
                        idx += 1;
                    }
                    None => {
                        // 末尾槽位（无后续字面）：
                        // 封闭值表 → 在剩余文本中查找任一值子串（取最早出现，忽略其后后缀）；
                        // 开放槽位（值表空）→ 取剩余全文。
                        let values = slot_values.get(name);
                        let has_values = values.map(|v| !v.is_empty()).unwrap_or(false);
                        if !has_values {
                            let value: String = chars[cur..].iter().collect();
                            result.insert(name.clone(), value);
                            cur = chars.len();
                            idx += 1;
                        } else {
                            let mut found: Option<(usize, usize)> = None;
                            for v in values.unwrap() {
                                let vchars: Vec<char> = v.chars().collect();
                                if let Some(rel) = find_subseq(&chars[cur..], &vchars) {
                                    let s = cur + rel;
                                    let e = s + vchars.len();
                                    if found.map_or(true, |(fs, _)| s < fs) {
                                        found = Some((s, e));
                                    }
                                }
                            }
                            match found {
                                Some((s, e)) => {
                                    let value: String = chars[s..e].iter().collect();
                                    result.insert(name.clone(), value);
                                    cur = e;
                                    idx += 1;
                                }
                                None => {
                                    if slot_optional.get(name).copied().unwrap_or(false) {
                                        result.insert(name.clone(), String::new());
                                        cur = chars.len();
                                        idx += 1;
                                    } else {
                                        return None;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Some(result)
}
