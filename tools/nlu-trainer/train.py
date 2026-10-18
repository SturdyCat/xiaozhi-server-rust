# NLU 意图分类训练脚本（容器内运行）。
#
# 输入：/work/input.json —— 服务端从 [nlu] 配置导出：
#   {"intents": [{"name": ..., "templates": ["把{room}的灯{action}", ...]}],
#    "slots":   [{"name": "room", "values": ["客厅", "卧室"], "optional": false}]}
# 输出（/work/output/）：model.onnx（int8 动态量化）、tokenizer.json、labels.txt。
#
# 语料策略（零外部依赖，全部从配置模板机械展开）：
#   1) 正样本：模板 {slot} 用值表代入（笛卡尔积封顶），开放槽位用少量代词/名词填充；
#   2) 负样本：①交叉歧义——A 技能模板代入 B 技能的槽位值；②兜底类——闲聊/疑问句
#      归入 other 标签，让模型学会"不属于任何技能"的拒识；
#   3) 标签序 = labels.txt 行序，服务端 [nlu].bert_label_map 把标签映射回技能名。
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

random.seed(SEED)

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


def expand_template(template, slots, cap):
    """把模板里的 {slot} 全部代入，返回句子列表（笛卡尔积封顶 cap）。"""
    import re

    names = re.findall(r"\{(\w+)\}", template)
    pools = [fill_values(n, slots) for n in names]
    if not pools:
        return [template]
    out = []
    for combo in itertools.product(*pools):
        sentence = template
        for name, value in zip(names, combo):
            sentence = sentence.replace("{" + name + "}", value)
        out.append(sentence)
        if len(out) >= cap:
            break
    return out


def build_corpus(intents, slots):
    texts, labels = [], []
    all_values = {n: {"name": n, "values": fill_values(n, slots)} for n in slots}
    for intent in intents:
        positives = []
        for tpl in intent.get("templates", []):
            positives.extend(expand_template(tpl, slots, cap=40))
        # 交叉歧义负样本：本技能模板代入**其他技能**的槽位值。
        cross = []
        for tpl in intent.get("templates", []):
            cross.extend(expand_template(tpl, all_values, cap=40))
        random.shuffle(positives)
        positives = positives[:PER_INTENT]
        texts.extend(positives)
        labels.extend([intent["name"]] * len(positives))
        # 负样本量 ≈ 正样本的一半：与本技能标签对立。
        random.shuffle(cross)
        cross = cross[: max(1, len(positives) // 2)]
        texts.extend(cross)
        labels.extend(["other"] * len(cross))
    texts.extend(OTHER_CORPUS)
    labels.extend(["other"] * len(OTHER_CORPUS))
    return texts, labels


def main():
    intents, slots = load_input()
    intent_names = [i["name"] for i in intents]
    label_list = intent_names + ["other"]
    print(f"[train] intents={intent_names} + other")

    texts, labels = build_corpus(intents, slots)
    order = list(range(len(texts)))
    random.shuffle(order)
    texts = [texts[i] for i in order]
    labels = [labels[i] for i in order]
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
    split = ds.train_test_split(test_size=0.1, seed=SEED)
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
        train_dataset=split["train"],
        eval_dataset=split["test"],
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
