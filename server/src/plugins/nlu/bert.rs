//! TinyBERT 意图分类器（技能级联第二级的语义模型实现，feature `nlu`）。
//!
//! ## 定位（级联口径）
//!
//! 终端 → ASR → 规则 NLU（命中即执行）→ **本模型 / 内置启发式打分**（达阈值即执行）
//! → 云 LLM → TTS。配置了 `[nlu].bert_enabled=true` 时本模型**先于**内置启发式
//! （[`crate::plugins::skill::LightModel`]）运行：BERT 给出的是真实语义相似度，
//! 对"说得完全不一样但意思一样"的指令比字面锚点鲁棒；本模型未配置 / 加载失败 /
//! 未达阈值时仍回落到启发式与 LLM，级联语义不变（漏触发优于误触发）。
//!
//! ## 模型与输入输出约定
//!
//! - 模型：int8 量化的 TinyBERT4（4 层 / hidden 312，约 14.5M 参数），HF
//!   `transformers` 导出的单文件 `model.onnx`（分类头 `num_labels` = 标签数）；
//! - 分词：配套 `vocab.txt`，`[CLS] text [SEP]`，超长按 [`NluConfig::bert_max_len`] 截断；
//! - 输出：logits `[1, num_labels]`，softmax 后取最大者为（标签, 置信度）；
//!   输入名按模型实际声明自适应（`input_ids` / `attention_mask` 必需，
//!   导出图若带 `token_type_ids` 则补全零向量）。
//!
//! ## 为什么标签不直接当技能名
//!
//! 训练标签是模型侧命名（英文短横线等），技能名是服务端目录命名——
//! [`NluConfig::bert_label_map`] 一处映射，未映射的同名直配；映射后目录里不存在
//! 的技能由 [`crate::plugins::skill::SkillRegistry::resolve`] 拒绝（只提名不执行）。

use std::sync::Arc;

use anyhow::{bail, Result};

use crate::plugins::nlu::NluConfig;

/// 一次分类的结果（延迟拆解：分词与推理分开计时，测试台逐项展示）。
#[derive(Debug, Clone, PartialEq)]
pub struct BertHit {
    /// 置信度最高的 ONNX 标签（**未经** `bert_label_map` 映射的原文）。
    pub label: String,
    /// softmax 后最高标签置信度（[0,1]）。
    pub confidence: f32,
    /// 分词耗时（ms）。
    pub tokenize_ms: u64,
    /// ONNX 推理耗时（ms）。
    pub infer_ms: u64,
}

/// TinyBERT 分类器抽象：让级联与测试台不依赖 `nlu` feature（无 feature 时无实现可注入）。
pub trait BertClassify: Send + Sync {
    /// 对一句话分类；单句毫秒级，调用方**不进**推理闸门（与缓存命中同口径）。
    fn classify_timed(&self, text: &str) -> Result<BertHit>;
}

/// TinyBERT 层的配置签名：路径 / 阈值 / 线程 / 截断 / 标签映射任一变化即重建
/// （跨会话缓存的失效键，见 [`crate::engine::Engines`]）。
pub fn bert_signature(nlu: &NluConfig) -> String {
    let mut pairs: Vec<String> = nlu
        .bert_label_map
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    pairs.sort();
    format!(
        "bert|model={}|vocab={}|labels={}|threshold={}|threads={}|max_len={}|map={}",
        nlu.bert_model,
        nlu.bert_vocab,
        nlu.bert_labels,
        nlu.bert_threshold,
        nlu.bert_num_threads,
        nlu.bert_max_len,
        pairs.join(",")
    )
}

/// 本次编译是否带 TinyBERT 引擎（无 feature 时配置了 `[nlu].bert_enabled` 的错误文案
/// 由调用方给"怎么修"：用 `--features nlu` 重新编译）。
pub fn bert_feature_enabled() -> bool {
    cfg!(feature = "nlu")
}

/// 按配置构建 TinyBERT 分类器（跨会话共享实例的工厂；失败文案自带"怎么修"）。
///
/// - 未启用 `nlu` feature → 明确报"用 `--features nlu` 重新编译"；
/// - 路径未配置 / 文件不存在 → 报缺哪个文件、去哪配。
pub fn build_bert_checked(nlu: &NluConfig) -> Result<Arc<dyn BertClassify>> {
    if !nlu.bert_configured() {
        bail!(
            "[nlu] TinyBERT 未配置完整：请把 [nlu].bert_enabled 设为 true，\
             并填写 bert_model / bert_vocab / bert_labels 三个文件路径"
        );
    }
    if !bert_feature_enabled() {
        bail!(
            "[nlu] 配置了 TinyBERT（bert_enabled=true），但本次编译未启用 nlu feature：\
             请用 `cargo build --features nlu` 重新编译服务端"
        );
    }
    #[cfg(feature = "nlu")]
    {
        use std::path::Path;
        for (key, path) in [
            ("bert_model", &nlu.bert_model),
            ("bert_vocab", &nlu.bert_vocab),
            ("bert_labels", &nlu.bert_labels),
        ] {
            if !Path::new(path.trim()).is_file() {
                bail!(
                    "[nlu].{key} 指向的文件不存在：{path}，请检查路径（容器内注意挂载），\
                     或暂时把 [nlu].bert_enabled 改回 false"
                );
            }
        }
        build_bert(nlu)
    }
    #[cfg(not(feature = "nlu"))]
    {
        unreachable!("bert_feature_enabled() 为 true 必然带 nlu feature 编译")
    }
}

/// 按配置构建（feature `nlu` 专享；调用方先过 [`build_bert_checked`] 的可读错误）。
#[cfg(feature = "nlu")]
pub fn build_bert(nlu: &NluConfig) -> Result<Arc<dyn BertClassify>> {
    Ok(Arc::new(TinyBertNlu::new(nlu)?))
}

#[cfg(feature = "nlu")]
mod engine {
    use std::borrow::Cow;

    use anyhow::{anyhow, bail, Context, Result};

    use super::{BertClassify, BertHit};
    use crate::plugins::nlu::NluConfig;

    /// 已加载的 TinyBERT 分类器：分词器 + ONNX 会话（只读，跨会话共享）。
    pub struct TinyBertNlu {
        tokenizer: tokenizers::Tokenizer,
        /// `Session::run` 需要 `&mut`；推理互斥由这层锁承载（单句毫秒级）。
        session: std::sync::Mutex<ort::session::Session>,
        labels: Vec<String>,
        /// 模型输入图里是否声明了 token_type_ids（BERT 导出常见；按需补零输入）。
        has_token_type_ids: bool,
        max_len: usize,
    }

    impl TinyBertNlu {
        pub fn new(nlu: &NluConfig) -> Result<Self> {
            let tokenizer = tokenizers::Tokenizer::from_file(&nlu.bert_vocab).map_err(|e| {
                anyhow!(
                    "[nlu] vocab.txt 加载失败（{}）: {e}；请确认与导出模型配套的 BERT 词表",
                    nlu.bert_vocab
                )
            })?;
            let labels: Vec<String> = std::fs::read_to_string(&nlu.bert_labels)
                .context("[nlu] labels 文件读取失败")?
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            if labels.is_empty() {
                bail!("[nlu] labels 文件为空：请一行一个标签（行号 = 分类输出下标）");
            }
            let builder = ort::session::Session::builder()
                .map_err(|e| anyhow!("[nlu] ONNX 会话构建器创建失败: {e}"))?;
            let builder = builder
                .with_intra_threads(nlu.bert_num_threads.max(1))
                .map_err(|e| anyhow!("[nlu] ONNX 线程数（bert_num_threads）设置失败: {e}"))?;
            let session = builder.commit_from_file(&nlu.bert_model).map_err(|e| {
                anyhow!(
                    "[nlu] ONNX 模型加载失败（{}）: {e}；请确认是 HF 导出的分类模型",
                    nlu.bert_model
                )
            })?;
            let has_token_type_ids = session.inputs.iter().any(|i| i.name == "token_type_ids");
            if nlu.bert_max_len < 3 {
                bail!("[nlu].bert_max_len 至少为 3（[CLS] + 正文 + [SEP]）");
            }
            Ok(TinyBertNlu {
                tokenizer,
                session: std::sync::Mutex::new(session),
                labels,
                has_token_type_ids,
                max_len: nlu.bert_max_len,
            })
        }
    }

    impl BertClassify for TinyBertNlu {
        fn classify_timed(&self, text: &str) -> Result<BertHit> {
            let t0 = std::time::Instant::now();
            // add_special_tokens=true 产生 [CLS] … [SEP]；超长按 max_len 截断（含特殊符）。
            let enc = self
                .tokenizer
                .encode(text, true)
                .map_err(|e| anyhow!("[nlu] 分词失败: {e}"))?;
            let mut ids: Vec<u32> = enc.get_ids().to_vec();
            ids.truncate(self.max_len);
            let len = ids.len();
            let ids: Vec<i32> = ids.iter().map(|&i| i as i32).collect();
            let mask: Vec<i32> = enc
                .get_attention_mask()
                .iter()
                .take(len)
                .map(|&m| m as i32)
                .collect();
            let tokenize_ms = t0.elapsed().as_millis() as u64;

            let t1 = std::time::Instant::now();
            // 形状 [1, len]（batch=1 的动态维度）；from_array 接受 i64 形状 + i32 数据。
            let shape = [1i64, len as i64];
            let ids = ort::value::Tensor::from_array((shape, ids))?;
            let mask = ort::value::Tensor::from_array((shape, mask))?;
            let mut inputs: Vec<(Cow<'_, str>, ort::session::SessionInputValue<'_>)> = vec![
                ("input_ids".into(), ids.into()),
                ("attention_mask".into(), mask.into()),
            ];
            if self.has_token_type_ids {
                let types = ort::value::Tensor::from_array((shape, vec![0i32; len]))?;
                inputs.push(("token_type_ids".into(), types.into()));
            }
            let mut session_guard = self.session.lock().unwrap_or_else(|e| e.into_inner());
            let outputs = session_guard.run(inputs)?;
            // logits: [1, num_labels] 的 f32 张量；softmax 后取 argmax。
            let (_logits_shape, logits) = outputs[0].try_extract_tensor::<f32>()?;
            let (_best, conf) = softmax_argmax(logits);
            let infer_ms = t1.elapsed().as_millis() as u64;

            // 标签下标 = argmax 下标；越界（标签表比模型输出短）按"未注册意图"上报原文下标。
            let idx = argmax(logits);
            let label = self
                .labels
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("#label_{idx}"));
            Ok(BertHit {
                label,
                confidence: conf,
                tokenize_ms,
                infer_ms,
            })
        }
    }

    /// 单行 logits → (argmax 下标, softmax 后最大概率)。
    fn softmax_argmax(logits: &[f32]) -> (usize, f32) {
        let idx = argmax(logits);
        if logits.is_empty() {
            return (0, 0.0);
        }
        let max = logits[idx];
        let exps: Vec<f32> = logits.iter().map(|&v| (v - max).exp()).collect();
        let sum: f32 = exps.iter().sum();
        if sum <= 0.0 {
            return (idx, 0.0);
        }
        (idx, exps[idx] / sum)
    }

    fn argmax(v: &[f32]) -> usize {
        let mut best = 0usize;
        for (i, &x) in v.iter().enumerate() {
            if x > v[best] {
                best = i;
            }
        }
        best
    }
}

#[cfg(feature = "nlu")]
pub use engine::TinyBertNlu;
