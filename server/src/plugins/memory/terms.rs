//! 词项抽取：把中文/英文文本切成**可检索的词项**（记忆的本地词法召回用）。
//!
//! ## 为什么自己切（而不是用 FTS5）
//!
//! 上游 graph-memory 没有给"轮次摘要"建 FTS5，词法回退是 `summary LIKE '%完整短语%'`
//! ——要求**完整短语**命中，中文口语化提问几乎召不回（见
//! `docs/soul-and-graph-memory-plan.md` §2.4 缺陷 2）。
//!
//! 我们不用 SQLite 的 FTS5 分词器，原因是中文分词在无词典/无 embedding 时不可靠：
//! - `unicode61` 把整个中文串当**一个 token** → 子串查询不命中；
//! - `trigram` 要求查询 ≥3 字，中文两字词（"餐厅"/"上次"）直接失配，且 3 字窗口
//!   换个说法就完全不同（"那家餐厅" 与 "个餐厅在" 无交集）。
//!
//! 因此改成**自己建倒排索引**：文本 → 汉字 **二元组（bigram）** + ASCII 单词（小写），
//! 召回时按"命中的不同词项数"排序。二元组对中文的召回率远高于 3-gram/LIKE，
//! 代价是索引行数约为字数的 2 倍（几千轮对话量级完全可接受）。
//!
//! 规范化只做 trim + 空白折叠 + 小写，**绝不改写语义**（对齐上游刻意为之的保守策略）。

/// 单个词项的最大长度（ASCII 单词超长截断，避免索引里出现整段 URL）。
const MAX_TERM_LEN: usize = 24;
/// 一次查询最多取多少个词项（限制 SQL 参数个数与噪声词占比）。
pub const MAX_QUERY_TERMS: usize = 24;

/// 是否 CJK 字符（含中日韩统一表意文字 + 扩展 A + 兼容区 + 假名 + 谚文）。
///
/// 只用于**分词**（决定是否做二元组），因此按大致区间判断即可。
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF     // 扩展 A
        | 0x4E00..=0x9FFF   // 基本区
        | 0xF900..=0xFAFF   // 兼容表意文字
        | 0x3040..=0x30FF   // 平假名/片假名
        | 0xAC00..=0xD7AF   // 谚文音节
    )
}

/// 抽取文本的**索引词项**（摘要入库用）。
pub fn index_terms(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cjk_run: Vec<char> = Vec::new();
    let mut ascii_run = String::new();

    let flush_cjk = |run: &mut Vec<char>, out: &mut Vec<String>| {
        if run.len() == 1 {
            // 单字不成词（"我"/"的"这类噪声太多）；但保留单字 run 里的字符不做词项
        } else {
            for w in run.windows(2) {
                out.push(w.iter().collect::<String>());
            }
        }
        run.clear();
    };
    let flush_ascii = |run: &mut String, out: &mut Vec<String>| {
        if run.chars().count() >= 2 {
            out.push(truncate_chars(&run.to_lowercase(), MAX_TERM_LEN));
        }
        run.clear();
    };

    for c in text.chars() {
        if is_cjk(c) {
            flush_ascii(&mut ascii_run, &mut out);
            cjk_run.push(c);
        } else if c.is_alphanumeric() {
            flush_cjk(&mut cjk_run, &mut out);
            ascii_run.push(c);
        } else {
            flush_cjk(&mut cjk_run, &mut out);
            flush_ascii(&mut ascii_run, &mut out);
        }
    }
    flush_cjk(&mut cjk_run, &mut out);
    flush_ascii(&mut ascii_run, &mut out);

    dedup(&mut out);
    out
}

/// 抽取**查询**词项：与索引同构（同构才能命中），并对过长文本设上限。
///
/// 超限时**保留靠前的词项**：中文提问的关键信息通常在前半句，且标点/语气词
/// 多在后半句（"你觉得呢"/"可以吗"）。
pub fn query_terms(text: &str) -> Vec<String> {
    let mut terms = index_terms(text);
    terms.truncate(MAX_QUERY_TERMS);
    terms
}

/// 去重并保持顺序（二元组重复很常见："哈哈哈哈哈"）。
fn dedup(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|t| !t.is_empty() && seen.insert(t.clone()));
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests;
