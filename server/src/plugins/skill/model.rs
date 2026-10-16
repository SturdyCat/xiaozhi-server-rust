//! 轻量级本地 NLU 模型（级联第二级）：规则未命中后的**零外网**模糊打分层。
//!
//! ## 定位（级联口径）
//!
//! 终端 → ASR → **规则 NLU**（命中即执行）→ **本模型**（快速打分，达阈值立即执行）
//! → 云 LLM（识别为 skill 则回交目录执行，否则正常聊天）→ TTS。
//! 本模型只在规则层未命中时运行，目标是把"说得不够工整"的指令在本地就地消化，
//! 省掉一次云 LLM 往返；不确定的一律放行给 LLM（漏触发优于误触发）。
//!
//! ## 为什么不做分词（预分词评估结论）
//!
//! 指令模板的特征是**字面锚点 + 封闭槽位值**，两者都是短字符序列；对归一化文本做
//! 有序子序列定位即可覆盖"多说了字 / 少说了字 / 同义前后缀"的常见 ASR 抖动。
//! 引入 jieba 类分词要背词典（MB 级）且分词边界错误反而伤害锚点匹配——**不做**。
//!
//! ## 打分口径（确定性、可单测）
//!
//! 对每条"模板→技能"编译出锚点序列（字面 token，[`parse_template`] 已归一化）：
//!
//! 1. **覆盖分** `coverage`：锚点按序不重叠地在文本中定位（贪心取最早命中），
//!    命中字符数 ÷ 锚点总字符数；任一锚点完全找不到 → 该模板弃权；
//! 2. **跨度惩罚**：首末锚点之间**既非锚点也非已认可槽位**的游离字符越多越可疑
//!    （指令中间不该夹无关话），`score = coverage − 0.25 × extra/span`——
//!    槽位间隙里的礼貌 filler（"帮我/一下"）算槽位区域，不惩罚；
//! 3. **封闭槽位三级判定**（值表是封闭枚举，这是模糊层真正的放宽点）：
//!    - 间隙**恰好等于**某值表成员 → 认可，+0.05；
//!    - 否则间隙**恰好包含唯一一个**成员（如"帮我打开一下"含"打开"）→ 认可并取该值，
//!      +0.02；间隙含**多个**成员（歧义）或含否定词（"别/不要/不用…"）→ 整条弃权
//!      ——方向性副作用宁可漏给 LLM，绝不能反向执行；
//!    - 一个成员都不含 → 弃权；
//! 4. **接受条件**：最高分 ≥ `threshold` 且领先**其他技能**最高分 ≥ `margin`
//!    （相近指令的歧义句不放行，交给 LLM 兜底）。
//!
//! 本模型只**提名**（技能名 + 槽位候选）；动作合法性、开放槽位长度、可选性等仍由
//! [`super::SkillRegistry::resolve`] 统一把关——模型永远不能直接产生副作用。
//!
//! 纯槽位模板（无字面锚点）不进本模型：没有锚点就没有"模糊"可言，
//! 那类模板交给规则层的确定性匹配。

use std::collections::HashMap;

use crate::plugins::nlu::{contains_negation, norm, parse_template, template::Tok};

/// 跨度内游离字符（非锚点非槽位）的惩罚系数（见模块文档打分口径 2）。
const SPAN_PENALTY: f32 = 0.25;
/// 封闭槽位精确命中的加分（见打分口径 3）。
const SLOT_EXACT_BONUS: f32 = 0.05;
/// 封闭槽位"包含唯一成员"的加分（弱于精确命中）。
const SLOT_CONTAIN_BONUS: f32 = 0.02;
/// 槽位加分上限（防止多槽位模板无脑越过阈值）。
const SLOT_BONUS_CAP: f32 = 0.15;

/// 一次模型提名的结果（动作/槽位仍需目录 `resolve` 复核）。
#[derive(Debug, Clone, PartialEq)]
pub struct ModelHit {
    pub skill: String,
    pub slots: HashMap<String, String>,
    /// 最高分（[0,1]，已含惩罚与加分）。
    pub score: f32,
    /// 其他技能的最高分（观测用：`score - runner_up` 应 ≥ margin）。
    pub runner_up: f32,
}

/// 锚点间隙的槽位：名字 + 封闭值表（空 = 开放槽位，不做成员校验）。
#[derive(Debug, Clone)]
struct Gap {
    name: String,
    values: Vec<String>,
}

/// 一条编译后的模板：锚点序列 + 每个锚点**之前**的间隙槽位。
#[derive(Debug, Clone)]
struct Entry {
    skill: String,
    /// 字面锚点（已归一化、非空、按模板顺序）。
    anchors: Vec<String>,
    /// `gaps[i]` = 第 i 个锚点之前的文本（首锚点前 / 锚点之间 / 末锚点后各一段）。
    gaps: Vec<Option<Gap>>,
    /// 锚点字符总数（覆盖分母）；恒 > 0（纯槽位模板不编译）。
    total: usize,
}

/// 已编译的轻量模型：全部技能模板的打分索引（会话内只读，构造一次）。
#[derive(Debug, Clone, Default)]
pub struct LightModel {
    entries: Vec<Entry>,
    threshold: f32,
    margin: f32,
}

impl LightModel {
    /// 从（技能名 → 模板串）与（槽位名 → 原始值表）编译。
    ///
    /// 值表成员在编译时统一 [`norm`] 归一化（与规则层同口径），打分时对间隙文本
    /// 也只 `norm` 一次即可做精确成员判定。
    pub fn build(
        skills: &[(String, Vec<String>)],
        slots: &HashMap<String, Vec<String>>,
        threshold: f32,
        margin: f32,
    ) -> Self {
        let mut entries = Vec::new();
        for (skill, templates) in skills {
            for t in templates {
                let mut anchors: Vec<String> = Vec::new();
                let mut gaps: Vec<Option<Gap>> = Vec::new();
                let mut pending: Option<String> = None;
                for tok in parse_template(t).tokens {
                    match tok {
                        Tok::Lit(s) if !s.is_empty() => {
                            gaps.push(close_gap(pending.take(), slots));
                            anchors.push(s);
                        }
                        Tok::Lit(_) => {} // 纯标点锚点被 norm 吃掉：跳过，不留空锚
                        Tok::Slot(name) => pending = Some(name),
                    }
                }
                gaps.push(close_gap(pending.take(), slots));
                if anchors.is_empty() {
                    continue; // 纯槽位/空模板：不进模糊层（规则层确定性匹配）
                }
                let total = anchors.iter().map(|a| a.chars().count()).sum();
                entries.push(Entry {
                    skill: skill.clone(),
                    anchors,
                    gaps,
                    total,
                });
            }
        }
        // 非有限阈值按"关闭该级"处置（fail-safe）：validate 会在保存期拦下并提示，
        // 运行期这里不再给"沾边就执行"留任何口子。
        let safe = |v: f32| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                1.0
            }
        };
        LightModel {
            entries,
            threshold: safe(threshold),
            margin: safe(margin),
        }
    }

    /// 模型是否可能提名（无条目或阈值封顶时恒不命中，调用方可跳过该阶段）。
    pub fn is_active(&self) -> bool {
        !self.entries.is_empty() && self.threshold < 1.0
    }

    /// 打分并提名：接受条件见模块文档；不接受返回 `None`（调用方放行 LLM）。
    pub fn parse(&self, text: &str) -> Option<ModelHit> {
        if !self.is_active() {
            return None;
        }
        let u = norm(text);
        if u.is_empty() {
            return None;
        }
        let chars: Vec<char> = u.chars().collect();
        // 每技能取最高分；同分取字典序更小的技能名（确定性，不依赖编译顺序）。
        let mut best: Option<(f32, &Entry, HashMap<String, String>)> = None;
        let mut runner_up = 0.0f32;
        for e in &self.entries {
            let Some((score, slots)) = score_entry(&chars, e) else {
                continue;
            };
            let take = match &best {
                None => true,
                Some((bs, be, _)) => score > *bs || (score == *bs && e.skill < be.skill),
            };
            if take {
                // 原最高分若属于**其他**技能，落为 runner_up 候选。
                if let Some((bs, be, _)) = &best {
                    if be.skill != e.skill {
                        runner_up = runner_up.max(*bs);
                    }
                }
                best = Some((score, e, slots));
            } else {
                // 本条未超过现有最高：若属其他技能，参与 runner_up（不超过最高分，防同技能刷高）。
                let top = best.as_ref().map_or(0.0, |(bs, _, _)| *bs);
                runner_up = runner_up.max(score.min(top));
            }
        }
        let (score, entry, slots) = best?;
        if score < self.threshold || score - runner_up < self.margin {
            return None;
        }
        Some(ModelHit {
            skill: entry.skill.clone(),
            slots,
            score,
            runner_up,
        })
    }
}

/// 单模板打分：`None` = 锚点缺失或封闭槽位值不匹配（整条弃权）；`Some` = (分数, 槽位)。
fn score_entry(u: &[char], e: &Entry) -> Option<(f32, HashMap<String, String>)> {
    // 有序贪心定位：每个锚点从上一命中终点之后找最早出现位置（不重叠、保顺序）。
    let mut positions: Vec<(usize, usize)> = Vec::with_capacity(e.anchors.len());
    let mut cur = 0usize;
    for a in &e.anchors {
        let needle: Vec<char> = a.chars().collect();
        let rel = find_subseq(&u[cur..], &needle)?;
        let start = cur + rel;
        positions.push((start, start + needle.len()));
        cur = start + needle.len();
    }
    let matched: usize = positions.iter().map(|(s, t)| t - s).sum();
    let span = positions.last().map_or(0, |(_, t)| *t) - positions[0].0;
    let mut score = matched as f32 / e.total.max(1) as f32;
    // 槽位抽取 + 封闭值表三级判定：gaps[i] 在锚点 i 之前 = 上锚点终点到本锚点起点之间。
    let mut slots = HashMap::new();
    let mut bonus = 0.0f32;
    let mut slot_chars = 0usize; // 已认可槽位占用的字符数（从跨度惩罚里豁免）
    for (i, gap) in e.gaps.iter().enumerate() {
        let Some(gap) = gap else { continue };
        let from = i
            .checked_sub(1)
            .and_then(|k| positions.get(k))
            .map_or(0, |(_, t)| *t);
        let to = positions.get(i).map_or(u.len(), |(s, _)| *s);
        let raw: String = u[from..to].iter().collect();
        let v = norm(&raw);
        if gap.values.is_empty() {
            // 开放槽位：取归一化原文（合法性——如长度上限——由 resolve 复核）。
            slots.insert(gap.name.clone(), v);
            slot_chars += raw.chars().count();
            continue;
        }
        if let Some(exact) = gap.values.iter().find(|x| **x == v) {
            slots.insert(gap.name.clone(), exact.clone());
            bonus += SLOT_EXACT_BONUS;
        } else {
            // 模糊放宽：间隙包含恰好一个值表成员才认可（"帮我打开一下"含"打开"）；
            // 多成员 = 歧义，含否定词 = 方向相反，都弃权放行 LLM（否定词表与规则层同源）。
            let contains: Vec<&String> = gap
                .values
                .iter()
                .filter(|x| v.contains(x.as_str()))
                .collect();
            if contains.len() != 1 || contains_negation(&v) {
                return None;
            }
            slots.insert(gap.name.clone(), contains[0].clone());
            bonus += SLOT_CONTAIN_BONUS;
        }
        slot_chars += raw.chars().count();
    }
    if span > 0 {
        let extra = span.saturating_sub(matched + slot_chars);
        score -= SPAN_PENALTY * extra as f32 / span as f32;
    }
    score = (score + bonus.min(SLOT_BONUS_CAP)).clamp(0.0, 1.0);
    Some((score, slots))
}

/// 在 `hay` 里找 `needle` 首次出现的字符下标（无则 `None`）；空 needle 恒 0。
fn find_subseq(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// 把"上一个槽位名"编译成锚点间隙（含归一化封闭值表）；`None` 表示无间隙。
fn close_gap(pending: Option<String>, slots: &HashMap<String, Vec<String>>) -> Option<Gap> {
    pending.map(|name| Gap {
        values: slots
            .get(&name)
            .map(|vs| vs.iter().map(|v| norm(v)).collect())
            .unwrap_or_default(),
        name,
    })
}
