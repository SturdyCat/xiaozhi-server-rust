//! 模型唯一 NLU 字段元数据。真实初始值来自NluConfig与JSON种子。

use super::{FieldKind, FieldSchema, HotReload};

pub(super) const NLU_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        key: "enabled", label: "启用技能目录", kind: FieldKind::Bool, default_hint: "true",
        help: "启用保存态技能目录；只有真实训练模型可选择意图。初始技能来自JSON种子，可编辑或显式清空。模板仅作训练例句及同技能抽槽，缺参澄清；没有规则、启发式或LLM重新识别。command目录独立启停，但也必须通过模型。",
        hot: HotReload::NextSession, required: true,
    },
    FieldSchema {
        key: "bert_enabled", label: "启用模型意图分类", kind: FieldKind::Bool, default_hint: "false",
        help: "唯一意图识别开关，需feature nlu及模型/词表/标签文件。关闭时只允许普通聊天，无技能工具；启用但加载或推理失败时明确错误，不回退规则或LLM。构建候选不自动启用，须先独立完成拒识验收。",
        hot: HotReload::NextSession, required: true,
    },
    FieldSchema {
        key: "bert_model", label: "TinyBERT模型路径", kind: FieldKind::Path, default_hint: "",
        help: "ONNX模型文件，与词表及标签对应；bert_enabled=true时必填且文件存在。",
        hot: HotReload::NextSession, required: false,
    },
    FieldSchema {
        key: "bert_vocab", label: "TinyBERT词表路径", kind: FieldKind::Path, default_hint: "",
        help: "与模型配套的tokenizer vocab.txt，启用模型时必填。",
        hot: HotReload::NextSession, required: false,
    },
    FieldSchema {
        key: "bert_labels", label: "TinyBERT标签路径", kind: FieldKind::Path, default_hint: "",
        help: "每行一个标签，顺序必须与输出logits一致；other是保留拒识标签，不可改映射为动作。",
        hot: HotReload::NextSession, required: false,
    },
    FieldSchema {
        key: "bert_threshold", label: "模型接受阈值", kind: FieldKind::Float, default_hint: "0.75",
        help: "置信度达到阈值才接受同一技能；other/低分不执行。取值0~1的有限数，不能仅靠高阈值代替负例验收。",
        hot: HotReload::NextSession, required: false,
    },
    FieldSchema {
        key: "bert_num_threads", label: "模型推理线程数", kind: FieldKind::Int, default_hint: "1",
        help: "ONNX Runtime会话内线程数，默认1以减少与音频链路争用。",
        hot: HotReload::NextSession, required: false,
    },
    FieldSchema {
        key: "bert_max_len", label: "模型最大序列长度", kind: FieldKind::Int, default_hint: "64",
        help: "分词截断/填充长度，必须与导出模型一致。",
        hot: HotReload::NextSession, required: false,
    },
];
