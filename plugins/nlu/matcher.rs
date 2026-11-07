//! 仅为模型已经接受的同一技能抽取槽位；绝不遍历目录选择意图。
//! 完整消费模板，不接受模糊包含、前后缀猜测或多值候选。保留开放槽位原文（含时间冒号）。

use super::{
    norm,
    template::{Template, Tok},
};
use std::collections::HashMap;

pub(crate) fn match_template(
    text: &str,
    template: &Template,
    values: &HashMap<String, Vec<String>>,
    optional: &HashMap<String, bool>,
) -> Option<HashMap<String, String>> {
    let text = text
        .trim()
        .trim_end_matches(['。', '！', '？', '.', '!', '?'])
        .trim();
    let mut rest = text;
    let mut slots = HashMap::new();
    for (index, token) in template.tokens.iter().enumerate() {
        match token {
            Tok::Lit(literal) => rest = rest.strip_prefix(literal)?,
            Tok::Slot(name) => {
                let raw = match template.tokens.get(index + 1) {
                    Some(Tok::Slot(_)) => {
                        // 相邻槽位仅允许前槽是必填封闭枚举，且原文只有唯一边界。
                        // 例如 weekday=一 后接原文10点；开放时间之间绝不猜切分。
                        if optional.get(name).copied().unwrap_or(false) {
                            return None;
                        }
                        let allowed = values.get(name).filter(|v| !v.is_empty())?;
                        let mut prefixes = rest
                            .char_indices()
                            .skip(1)
                            .map(|(end, _)| &rest[..end])
                            .filter(|raw| allowed.contains(&norm(raw)));
                        let raw = prefixes.next()?;
                        if prefixes.next().is_some() {
                            return None;
                        }
                        // 不把一个连续十进制数拆成两个槽，如“周10点”不能猜作周1+0点。
                        if raw.ends_with(|c: char| c.is_ascii_digit())
                            && rest[raw.len()..].starts_with(|c: char| c.is_ascii_digit())
                        {
                            return None;
                        }
                        raw
                    }
                    Some(Tok::Lit(anchor)) => {
                        let (raw, _) = rest.split_once(anchor)?;
                        // 多个相同结束锚点意味着边界歧义，不猜测哪一个属于槽位。
                        if rest.matches(anchor).count() != 1 {
                            return None;
                        }
                        raw
                    }
                    None => rest,
                };
                let value = raw.trim();
                if value.is_empty() && !optional.get(name).copied().unwrap_or(false) {
                    return None;
                }
                if let Some(allowed) = values.get(name).filter(|v| !v.is_empty()) {
                    if !(value.is_empty() && optional.get(name).copied().unwrap_or(false))
                        && !allowed.contains(&norm(value))
                    {
                        return None;
                    }
                }
                if let Some(old) = slots.insert(name.clone(), value.to_string()) {
                    if old != value {
                        return None;
                    }
                }
                rest = &rest[raw.len()..];
            }
        }
    }
    rest.is_empty().then_some(slots)
}
