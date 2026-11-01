# NLU 意图分类训练脚本（容器内运行）。
#
# 输入：/work/input.json —— 服务端从 [nlu] 配置导出：
#   {"intents": [{"name": ..., "templates": ["把{room}的灯{action}", ...]}],
#    "slots":   [{"name": "room", "values": ["客厅", "卧室"], "optional": false}]}
# 输出（/work/output/）：model.onnx（int8 动态量化）、tokenizer.json、labels.txt。
#
# 语料策略（零外部依赖，全部从配置模板机械展开）：
#   1) 正样本：模板 {slot} 用值表代入（笛卡尔积封顶），开放槽位用少量代词/名词填充；
#   2) 负样本：明确取消整条请求的句子、闲聊/疑问句归入 other；不把正常槽位展开标成负例。
#      有界展开候选全局去重、跨意图同句拒绝；负例与候选正句碰撞时保留正句；
#   3) 标签序 = labels.txt 行序，服务端 [nlu].bert_label_map 把标签映射回技能名。
from collections import Counter
import itertools
import json
import os
import random

INPUT = os.environ.get("TRAIN_INPUT", "/work/input.json")
OUTDIR = os.environ.get("TRAIN_OUTPUT", "/work/output")
MODEL_NAME = os.environ.get("MODEL_NAME", "hfl/rbt3")  # 3 层中文 RoBERTa，int8 后约 40MB
PER_INTENT = int(os.environ.get("PER_INTENT", "160"))
EPOCHS = float(os.environ.get("EPOCHS", "6"))
MAX_LEN = int(os.environ.get("MAX_LEN", "64"))
SEED = 42
TEMPLATE_UNIQUE_CAP = 40
TEMPLATE_SCAN_LIMIT = 4000

# 开放槽位（无值表）的填充词：保持语义为"某个提醒内容/时间表达"。
OPEN_FILLERS = {
    "task": ["喝水", "开会", "起床", "吃药", "给妈妈打电话", "写周报", "关火", "取快递"],
    "delay": ["10分钟", "半小时", "一小时", "两分钟", "五分钟", "20分钟"],
    "time": ["八点", "明天早上七点", "下午三点", "晚上九点半", "后天上午十点"],
    "repeat": ["每天", "每周一", "工作日", "每周五", "每个月一号"],
}
GENERIC_FILLER = ["这件事", "那个事", "重要的事"]

OTHER_CORPUS = [
    "今天天气怎么样", "给我讲个笑话", "你好呀", "现在几点了", "明天会下雨吗",
    "把音量调大一点", "播放一首歌", "我想听新闻", "帮我算一下三乘五", "谢谢",
    "你是谁", "讲个故事吧", "北京有什么好玩的地方", "现在油价多少", "今晚吃什么好",
]


def load_input():
    with open(INPUT, encoding="utf-8") as f:
        data = json.load(f)
    slots = {s["name"]: s for s in data.get("slots", [])}
    return data.get("intents", []), slots


def fill_values(slot_name, slots):
    """槽位的候选填充值：封闭值表直用；开放槽位走代词表（兜底通用词）。"""
    slot = slots.get(slot_name)
    if slot and slot.get("values"):
        return list(slot["values"])
    return OPEN_FILLERS.get(slot_name, GENERIC_FILLER)


def expand_template(template, slots, cap, *, stats=None):
    """值表/句子去重后最多输出 cap 条；扫描另限额，不遍历完整笛卡尔积。"""
    import re

    if cap < 1 or TEMPLATE_SCAN_LIMIT < 1:
        raise ValueError("template output cap and scan limit must be positive")
    # 同一占位符多次出现仍表示同一个值，不能增加无效的组合维度。
    names = list(dict.fromkeys(re.findall(r"\{(\w+)\}", template)))
    pools = []
    for name in names:
        values = fill_values(name, slots)
        unique = list(dict.fromkeys(values))
        if stats is not None:
            stats["slot_value_duplicates_removed"] += len(values) - len(unique)
        pools.append(unique)

    combos = iter(itertools.product(*pools))
    out, seen = [], set()
    for combo in itertools.islice(combos, TEMPLATE_SCAN_LIMIT):
        if stats is not None:
            stats["expansion_combinations_scanned"] += 1
        sentence = template
        for name, value in zip(names, combo):
            sentence = sentence.replace("{" + name + "}", value)
        sentence = " ".join(sentence.split())
        if sentence in seen:
            if stats is not None:
                stats["expansion_duplicate_sentences_removed"] += 1
            continue
        seen.add(sentence)
        out.append(sentence)
        if len(out) >= cap:
            break
    # 仅探测有无剩余组合，不继续展开；截断范围之外不承诺冲突检查。
    sentinel = object()
    if next(combos, sentinel) is not sentinel and stats is not None:
        stats["expansion_templates_limited"] += 1
    return out


def build_corpus(intents, slots, *, stats=None):
    """生成无重复、无同句异标的语料；保留原来的 (texts, labels) 返回 API。

    冲突检查仅覆盖每模板有界展开的候选，但先于 PER_INTENT 抽样。
    取消必须针对整个请求；不能根据 task 中的“不/不要”推断取消意图。
    """
    if PER_INTENT < 1:
        raise ValueError("PER_INTENT must be positive")
    rng = random.Random(SEED)
    positives_by_text, positives_by_label = {}, {}
    counts = Counter()
    for intent in intents:
        name = intent["name"]
        if name == "other" or name in positives_by_label:
            raise ValueError(f"invalid or duplicate intent name: {name!r}")
        templates = intent.get("templates")
        if not isinstance(templates, list) or not templates:
            raise ValueError(f"intent {name!r} requires a non-empty templates list")
        positives_by_label[name] = []
        for tpl in templates:
            if not isinstance(tpl, str):
                raise ValueError(f"intent {name!r} requires string templates")
            for sentence in expand_template(tpl, slots, cap=TEMPLATE_UNIQUE_CAP, stats=counts):
                sentence = " ".join(sentence.split())
                if not sentence:
                    raise ValueError(f"empty training sentence for intent {name!r}")
                counts["positive_candidates"] += 1
                previous = positives_by_text.get(sentence)
                if previous is not None:
                    if previous != name:
                        raise ValueError(
                            f"conflicting positive labels {previous!r} / {name!r} "
                            f"for sentence {sentence!r}"
                        )
                    counts["positive_duplicates_removed"] += 1
                    continue
                positives_by_text[sentence] = name
                positives_by_label[name].append(sentence)
        if not positives_by_label[name]:
            raise ValueError(f"intent {name!r} has no unique positive examples")

    texts, labels = [], []
    negative_candidates = list(OTHER_CORPUS)
    for name, positives in positives_by_label.items():
        rng.shuffle(positives)
        selected = positives[:PER_INTENT]
        texts.extend(selected)
        labels.extend([name] * len(selected))
        # 显式撤销整条请求，避免“提醒我不要熬夜”这种合法任务被错标。
        cancelled = [f"取消这个请求，不要执行：{sentence}" for sentence in selected]
        rng.shuffle(cancelled)
        negative_candidates.extend(cancelled[: max(1, len(selected) // 2)])

    seen_negatives = set()
    for sentence in negative_candidates:
        sentence = " ".join(sentence.split())
        counts["negative_candidates"] += 1
        if sentence in positives_by_text:
            counts["negative_positive_collisions_skipped"] += 1
            continue
        if sentence in seen_negatives:
            counts["negative_duplicates_removed"] += 1
            continue
        seen_negatives.add(sentence)
        texts.append(sentence)
        labels.append("other")

    if stats is not None:
        stats.update({
            "positive_candidates": counts["positive_candidates"],
            "positive_duplicates_removed": counts["positive_duplicates_removed"],
            "positive_unique_before_cap": len(positives_by_text),
            "positive_selected": len(labels) - len(seen_negatives),
            "negative_candidates": counts["negative_candidates"],
            "negative_duplicates_removed": counts["negative_duplicates_removed"],
            "negative_positive_collisions_skipped": counts["negative_positive_collisions_skipped"],
            "negative_selected": len(seen_negatives),
            "label_counts": dict(Counter(labels)),
            "conflicting_texts": 0,
            "conflict_check_scope": "bounded_expanded_candidates_before_per_intent_sampling",
            "template_unique_cap": TEMPLATE_UNIQUE_CAP,
            "template_scan_limit": TEMPLATE_SCAN_LIMIT,
            "slot_value_duplicates_removed": counts["slot_value_duplicates_removed"],
            "expansion_combinations_scanned": counts["expansion_combinations_scanned"],
            "expansion_duplicate_sentences_removed": counts["expansion_duplicate_sentences_removed"],
            "expansion_templates_limited": counts["expansion_templates_limited"],
        })
    return texts, labels


def stratified_split_indices(labels, expected_labels, test_fraction=0.1):
    """每类 train/test 均非空且不重叠；不足两条直接拒绝，不复制样本。"""
    if not 0 < test_fraction < 1:
        raise ValueError("test_fraction must be between 0 and 1")
    by_label = {label: [] for label in expected_labels}
    if not by_label:
        raise ValueError("at least one label is required")
    for index, label in enumerate(labels):
        if label not in by_label:
            raise ValueError(f"unexpected corpus label: {label!r}")
        by_label[label].append(index)
    rng = random.Random(SEED)
    train_indices, test_indices = [], []
    for label, indices in by_label.items():
        if len(indices) < 2:
            raise ValueError(
                f"label {label!r} requires at least 2 unique examples for disjoint "
                f"train/test splits; got {len(indices)}"
            )
        rng.shuffle(indices)
        n_test = max(1, min(len(indices) - 1, round(len(indices) * test_fraction)))
        test_indices.extend(indices[:n_test])
        train_indices.extend(indices[n_test:])
    rng.shuffle(train_indices)
    rng.shuffle(test_indices)
    return train_indices, test_indices


def main():
    intents, slots = load_input()
    if not intents:
        raise ValueError("at least one intent is required for training")
    intent_names = [i["name"] for i in intents]
    label_list = intent_names + ["other"]
    print(f"[train] intents={intent_names} + other")

    stats = {}
    texts, labels = build_corpus(intents, slots, stats=stats)
    print(f"[train] corpus_stats={json.dumps(stats, ensure_ascii=False, sort_keys=True)}")
    train_indices, test_indices = stratified_split_indices(labels, label_list)
    split_stats = {
        "train": dict(Counter(labels[i] for i in train_indices)),
        "test": dict(Counter(labels[i] for i in test_indices)),
    }
    print(f"[train] split_stats={json.dumps(split_stats, ensure_ascii=False, sort_keys=True)}")
    print(f"[train] corpus={len(texts)} sentences, labels={label_list}")

    from datasets import Dataset
    from transformers import (
        AutoTokenizer,
        AutoModelForSequenceClassification,
        DataCollatorWithPadding,
        TrainingArguments,
        Trainer,
    )

    tokenizer = AutoTokenizer.from_pretrained(MODEL_NAME)
    model = AutoModelForSequenceClassification.from_pretrained(
        MODEL_NAME, num_labels=len(label_list)
    )
    ds = Dataset.from_dict({"text": texts, "label": [label_list.index(l) for l in labels]})

    def tok(batch):
        return tokenizer(batch["text"], truncation=True, max_length=MAX_LEN)

    ds = ds.map(tok, batched=True)
    args = TrainingArguments(
        output_dir="/work/ckpt",
        per_device_train_batch_size=32,
        learning_rate=5e-5,
        num_train_epochs=EPOCHS,
        eval_strategy="epoch",
        save_strategy="no",
        logging_steps=20,
        report_to=[],
        seed=SEED,
    )
    trainer = Trainer(
        model=model,
        args=args,
        train_dataset=ds.select(train_indices),
        eval_dataset=ds.select(test_indices),
        data_collator=DataCollatorWithPadding(tokenizer),
    )
    trainer.train()
    metrics = trainer.evaluate()
    print(f"[train] eval metrics: {metrics}")

    # 导出 ONNX（动态轴）→ int8 动态量化。
    from optimum.onnxruntime import ORTModelForSequenceClassification
    import onnxruntime as ort
    from onnxruntime.quantization import quantize_dynamic, QuantType

    os.makedirs(OUTDIR, exist_ok=True)
    export_dir = "/work/onnx-fp32"
    # optimum 的 from_pretrained 只接受路径（传内存对象会在 model_id.split 崩），
    # 训练完的模型先落盘再导出。
    final_dir = "/work/ckpt/final"
    # 新版 transformers/safetensors 保存时拒绝非连续张量，先统一转连续内存。
    for p in model.parameters():
        p.data = p.data.contiguous()
    model.save_pretrained(final_dir)
    tokenizer.save_pretrained(final_dir)
    ort_model = ORTModelForSequenceClassification.from_pretrained(
        final_dir, export=True
    )
    ort_model.save_pretrained(export_dir)
    onnx_path = os.path.join(export_dir, "model.onnx")
    quantized = os.path.join(OUTDIR, "model.onnx")
    quantize_dynamic(onnx_path, quantized, weight_type=QuantType.QInt8)

    tokenizer.save_pretrained(OUTDIR)  # tokenizer.json
    with open(os.path.join(OUTDIR, "labels.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(label_list) + "\n")
    print(f"[train] done → {quantized}")
    # 快速自检：量化模型在样例句上推理一遍。
    sess = ort.InferenceSession(quantized)
    sample = tokenizer("十分钟之后提醒我喝水", return_tensors="np")
    inputs = {k: v.astype("int64") for k, v in sample.items()}
    logits = sess.run(None, inputs)[0][0]
    print(f"[train] sample argmax={label_list[int(logits.argmax())]}")


if __name__ == "__main__":
    main()
