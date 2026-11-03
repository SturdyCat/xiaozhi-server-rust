# NLU 意图分类训练脚本（容器内运行）。
#
# 输入：/work/input.json —— 服务端从保存态统一技能目录导出（含启用 command 词表的 end_session）：
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
# 中间目录（checkpoint / fp32 导出）。容器内缺省不变；standalone（如 macOS 本机）
# 经 env 覆盖，避免要求根目录 /work。不影响语料、产物三件套与标签顺序契约。
CKPT_DIR = os.environ.get("TRAIN_CKPT", "/work/ckpt")
FP32_DIR = os.environ.get("TRAIN_FP32", "/work/onnx-fp32")

# 预训练底座：环境变量 MODEL_NAME 承载**规范名**（值语义不是 hub id）。
# 服务端按同一约定传参（server/src/gateway/ws/nlu_build/base.rs 的 BASE_MODEL），
# 两侧键集合由该文件的单测锁定，避免加了一边忘了另一边。
BASE_NAME = os.environ.get("MODEL_NAME", "minirbt-h288").strip()
# **当前唯一底座**：6 层 miniRBT（h288 词表）。体积/延迟更低，不代表更准。
# 表结构保留（而非写死常量）：服务端单测按源码解析这张表，锁 hub/lr/epochs 三字段
# 与服务端 BASE_MODEL 逐字段一致——只改一边会导致"日志说 h288、实际训的是别的"。
BASES = {
    "minirbt-h288": {"hub": "hfl/minirbt-h288", "lr": 1e-4, "epochs": 10.0},
}


def resolve_base(name):
    """规范名 → 底座 profile（hub + 微调超参）。

    未知名**直接拒绝**并列出允许值：绝不把任意字符串当 hub id 去下载，
    否则一个错拼的 MODEL_NAME 会静默训出另一个底座。
    """
    profile = BASES.get(name)
    if profile is None:
        raise ValueError(
            "unknown base model MODEL_NAME=%r; allowed: %s"
            % (name, ", ".join(sorted(BASES)))
        )
    # 返回副本：调用方改动超参不得污染 profile 原表。
    return dict(profile)


# 导入期就解析：模型名非法时容器在下载/训练之前失败，日志里只有一句明确原因。
BASE = resolve_base(BASE_NAME)
HUB_NAME = BASE["hub"]
# 超参允许 env 覆盖（服务端每次 build 都显式传）；缺省取底座 profile。
LEARNING_RATE = float(os.environ.get("LEARNING_RATE", str(BASE["lr"])))
EPOCHS = float(os.environ.get("EPOCHS", str(BASE["epochs"])))
PER_INTENT = int(os.environ.get("PER_INTENT", "160"))
MAX_LEN = int(os.environ.get("MAX_LEN", "64"))
SEED = 42
TEMPLATE_UNIQUE_CAP = 40
TEMPLATE_SCAN_LIMIT = 4000

# 开放槽位（无值表）的填充词：保持语义为"某个提醒内容/时间表达"。
OPEN_FILLERS = {
    "task": ["喝水", "开会", "起床", "吃药", "给妈妈打电话", "写周报", "关火", "取快递"],
    "delay": ["10分钟", "半小时", "一小时", "两分钟", "五分钟", "20分钟"],
    "interval": ["五分钟", "十分钟", "半小时", "一小时", "10分钟", "30分钟"],
    # 每日钟点不混入“明天/后天”，避免合成“每天明天早上七点”这类矛盾句。
    "daily_time": ["八点", "早上七点", "下午三点", "晚上九点半", "08:00", "21:30"],
    "time": ["八点", "明天上午十点", "下午三点", "晚上九点半", "后天上午十点", "大后天下午三点"],
    "weekday": ["一", "1", "二", "2", "五", "5", "日", "7"],
    "day": ["1", "一", "15", "十五", "28", "二十八", "31", "三十一"],
    "month": ["1", "一", "2", "二", "6", "六", "12", "十二"],
    "minute": ["0", "零", "5", "五", "20", "二十", "45", "四十五"],
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
    adjacent = re.findall(r"(?=\{(\w+)\}\{(\w+)\})", template)
    pools = []
    for name in names:
        values = fill_values(name, slots)
        unique = list(dict.fromkeys(values))
        if stats is not None:
            stats["slot_value_duplicates_removed"] += len(values) - len(unique)
        pools.append(unique)

    # 多槽模板先覆盖各维度，再补笛卡尔积，避免40条配额全耗在
    # 同一个月份/日期上。仅改变有界候选顺序，不增加展开预算。
    if len(pools) > 1:
        diagonal = dict.fromkeys(
            tuple(pool[i % len(pool)] for pool in pools)
            for i in range(min(cap, max(map(len, pools))))
        )
        combos = itertools.chain(
            diagonal,
            (combo for combo in itertools.product(*pools) if combo not in diagonal),
        )
    else:
        combos = iter(itertools.product(*pools))
    out, seen = [], set()
    for combo in itertools.islice(combos, TEMPLATE_SCAN_LIMIT):
        if stats is not None:
            stats["expansion_combinations_scanned"] += 1
        values = dict(zip(names, combo))
        if any(values[left].rstrip()[-1:].isdecimal() and
               values[right].lstrip()[:1].isdecimal() for left, right in adjacent):
            if stats is not None:
                stats["expansion_ambiguous_adjacent_values_skipped"] += 1
            continue
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


def smoke_cases(texts, labels, *, other_count=3, limit=2000):
    """冒烟自报：每个技能取 1 条**真实展开句**（不含 {slot} 花括号）+ 3 条 other 负例。

    服务端只用这些**文本**在它那边独立重跑判定，所以这里绝不能送模板原文：
    未展开的花括号句子等价于训练时没见过的输入，会把好模型判成"门禁失败"。

    **负例恒为 `other_count` 条，永不裁剪**：服务端要求负例正好 3 条且 3/3 全拒识，
    裁掉负例会让它把"数据不够"误报成"模型把闲聊句判成技能"（理由假的）。
    超预算只能裁**正例**（最少留 1 条），并在 `truncated` 里如实标记——
    服务端见到该标记会报 `smoke_truncated`，而不是拿被裁过的集合下判定。

    返回 `{"cases": [...], "truncated": bool}`。
    """
    picked, order = {}, []
    for text, label in zip(texts, labels):
        if not text.strip() or "{" in text or "}" in text:
            continue
        if label not in picked:
            picked[label] = []
            order.append(label)
        cap = other_count if label == "other" else 1
        if len(picked[label]) < cap and text not in picked[label]:
            picked[label].append(text)
    positives = [
        {"text": text, "expected": label}
        for label in order
        if label != "other"
        for text in picked.get(label, [])
    ]
    negatives = [{"text": text, "expected": "other"} for text in picked.get("other", [])]

    def encoded(cases, truncated):
        doc = {"cases": cases, "truncated": bool(truncated)}
        return json.dumps(doc, ensure_ascii=False, separators=(",", ":")).encode()

    truncated = False
    while len(positives) > 1 and len(encoded(positives + negatives, truncated)) > limit:
        positives.pop()
        truncated = True
    # 1 条正例 + 3 条负例仍超预算：不再砍负例（砍了就是"理由假的"），如实标记。
    if len(encoded(positives + negatives, truncated)) > limit:
        truncated = True
    return {"cases": positives + negatives, "truncated": truncated}


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
    # 底座证据必须早于 ML 依赖导入：依赖缺失/权重下载失败时，日志里也要能看出
    # 这次训练用的是哪个底座、哪组超参（否则失败日志无法归因）。
    print(f"[train] base={BASE_NAME} hub={HUB_NAME} lr={LEARNING_RATE} epochs={EPOCHS}")

    from transformers import (
        AutoTokenizer,
        AutoModelForSequenceClassification,
        DataCollatorWithPadding,
        TrainingArguments,
        Trainer,
    )

    # hfl/minirbt-h288 仓库只有 vocab.txt，没有 tokenizer.json；必须显式要 fast 分词器，
    # 让 transformers 走 slow→fast 自动转换产出 tokenizer.json。否则最终
    # tokenizer.save_pretrained 写不出 tokenizer.json，服务端三件套校验
    # （artifact.rs 的 FILES 白名单）会 fail closed，整次训练白跑。
    tokenizer = AutoTokenizer.from_pretrained(HUB_NAME, use_fast=True)
    model = AutoModelForSequenceClassification.from_pretrained(
        HUB_NAME, num_labels=len(label_list)
    )
    # 不用 datasets 库：直接批量分词成 list[dict]，再按切分下标取子集。
    # 与原来 Dataset.from_dict + .map + .select 等价，但省掉 pyarrow/pandas 等重依赖，
    # 显著缩小训练镜像体积。HF Trainer 接受 list[dict] 作为 dataset（默认 _remove_unused_columns
    # 会剔除模型 forward 不接受的多余列，这里本就不含 "text"）。
    label_ids = [label_list.index(l) for l in labels]
    enc = tokenizer(texts, truncation=True, max_length=MAX_LEN)
    features = []
    for i in range(len(texts)):
        ex = {key: enc[key][i] for key in enc.keys()}
        ex["label"] = label_ids[i]
        features.append(ex)
    args = TrainingArguments(
        output_dir=CKPT_DIR,
        per_device_train_batch_size=32,
        learning_rate=LEARNING_RATE,
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
        train_dataset=[features[i] for i in train_indices],
        eval_dataset=[features[i] for i in test_indices],
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
    export_dir = FP32_DIR
    # optimum 的 from_pretrained 只接受路径（传内存对象会在 model_id.split 崩），
    # 训练完的模型先落盘再导出。
    final_dir = os.path.join(CKPT_DIR, "final")
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
    # 冒烟自报（S3）：紧跟在 done 之后、sample 探针**之前**打印。
    # 位置理由：服务端只在日志尾 8192 字节里**从后向前**找这一行，既不能早到被
    # trainer 输出挤出窗口，也不能晚到被 STREAM 缓冲截断；紧跟 done 两头都稳。
    smoke = smoke_cases(texts, labels)
    print(f"[train] smoke={json.dumps(smoke, ensure_ascii=False, separators=(',', ':'))}")
    if smoke["truncated"]:
        # 服务端据此报 smoke_truncated（而不是误报"负例没拒识"）：裁剪过就不足以判定。
        print(
            f"[train] smoke truncated: {len(smoke['cases'])} cases kept within {2000} bytes"
        )
    # 快速自检：量化模型在样例句上推理一遍。
    sess = ort.InferenceSession(quantized)
    sample = tokenizer("十分钟之后提醒我喝水", return_tensors="np")
    inputs = {k: v.astype("int64") for k, v in sample.items()}
    logits = sess.run(None, inputs)[0][0]
    print(f"[train] sample argmax={label_list[int(logits.argmax())]}")


if __name__ == "__main__":
    main()
