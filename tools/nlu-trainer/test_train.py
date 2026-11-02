"""语料生成回归：只用 Python 标准库，不下载模型、不导入 torch。"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("nlu_train", Path(__file__).with_name("train.py"))
train = importlib.util.module_from_spec(spec)
spec.loader.exec_module(train)


class CorpusTests(unittest.TestCase):
    def assert_consistent(self, texts, labels):
        self.assertEqual(len(texts), len(labels))
        self.assertEqual(len(texts), len(set(texts)), "重复句会污染 train/test 切分")

    def test_positive_expansions_are_never_other(self):
        intents = [{"name": "reminder", "templates": ["提醒我{task}"]}]
        slots = {"task": {"values": ["喝水", "开会", "吃药", "取快递"]}}
        stats = {}
        texts, labels = train.build_corpus(intents, slots, stats=stats)
        self.assert_consistent(texts, labels)
        by_text = dict(zip(texts, labels))
        for task in slots["task"]["values"]:
            self.assertEqual(by_text[f"提醒我{task}"], "reminder")
        self.assertEqual(stats["conflicting_texts"], 0)
        self.assertEqual(stats["label_counts"]["reminder"], 4)
        self.assertEqual(stats["negative_selected"], len(train.OTHER_CORPUS) + 2)

    def test_deduplicates_templates_and_slot_values(self):
        intents = [{"name": "reminder", "templates": ["提醒我{task}", " 提醒我{task} "]}]
        slots = {"task": {"values": ["喝水", "喝水", "吃药"]}}
        stats = {}
        texts, labels = train.build_corpus(intents, slots, stats=stats)
        self.assert_consistent(texts, labels)
        self.assertEqual(stats["positive_candidates"], 4)
        self.assertEqual(stats["positive_duplicates_removed"], 2)
        self.assertEqual(stats["slot_value_duplicates_removed"], 2)
        self.assertEqual(stats["positive_unique_before_cap"], 2)
        self.assertEqual(stats["positive_selected"], 2)

    def test_rejects_cross_intent_collision_before_sampling(self):
        intents = [
            {"name": "first", "templates": ["正例甲", "正例乙"]},
            {"name": "second", "templates": [" 正例甲 ", "正例乙"]},
        ]
        with patch.object(train, "PER_INTENT", 1):
            with self.assertRaisesRegex(ValueError, "conflicting positive labels.*first.*second"):
                train.build_corpus(intents, {})

    def test_other_collision_keeps_positive(self):
        text = train.OTHER_CORPUS[0]
        stats = {}
        texts, labels = train.build_corpus(
            [{"name": "weather", "templates": [text]}], {}, stats=stats
        )
        self.assert_consistent(texts, labels)
        self.assertEqual(dict(zip(texts, labels))[text], "weather")
        self.assertEqual(stats["negative_positive_collisions_skipped"], 1)

    def test_negated_task_is_positive_and_only_whole_request_is_cancelled(self):
        values = ["不要熬夜", "不吃甜食", "不要忘记带伞"]
        texts, labels = train.build_corpus(
            [{"name": "reminder", "templates": ["提醒我{task}"]}],
            {"task": {"values": values}},
        )
        self.assert_consistent(texts, labels)
        by_text = dict(zip(texts, labels))
        for value in values:
            self.assertEqual(by_text[f"提醒我{value}"], "reminder")
        cancellations = [text for text in texts if text.startswith("取消这个请求，不要执行：")]
        self.assertTrue(cancellations)
        for text in cancellations:
            self.assertEqual(by_text[text], "other")

    def test_cancelled_negative_cannot_overwrite_any_positive(self):
        cancelled = "取消这个请求，不要执行：提醒我喝水"
        stats = {}
        texts, labels = train.build_corpus([
            {"name": "reminder", "templates": ["提醒我喝水"]},
            {"name": "special", "templates": [cancelled]},
        ], {}, stats=stats)
        self.assert_consistent(texts, labels)
        self.assertEqual(dict(zip(texts, labels))[cancelled], "special")
        self.assertEqual(stats["negative_positive_collisions_skipped"], 1)

    def test_negative_duplicates_are_removed(self):
        stats = {}
        with patch.object(train, "OTHER_CORPUS", ["你好", " 你好 "]):
            texts, labels = train.build_corpus([], {}, stats=stats)
        self.assertEqual((texts, labels), (["你好"], ["other"]))
        self.assertEqual(stats["negative_duplicates_removed"], 1)

    def test_build_is_deterministic_and_cap_follows_deduplication(self):
        intents = [{"name": "reminder", "templates": ["提醒我{task}", "提醒我{task}"]}]
        slots = {"task": {"values": ["喝水", "吃药", "开会"]}}
        with patch.object(train, "PER_INTENT", 2):
            first = train.build_corpus(intents, slots)
            second = train.build_corpus(intents, slots)
        self.assertEqual(first, second)
        self.assertEqual(first[1].count("reminder"), 2)
        self.assert_consistent(*first)

    def test_rejects_reserved_duplicate_labels_and_empty_sentences(self):
        for intents in (
            [{"name": "other", "templates": ["正句"]}],
            [{"name": "same", "templates": ["甲"]}, {"name": "same", "templates": ["乙"]}],
            [{"name": "empty", "templates": ["   "]}],
        ):
            with self.subTest(intents=intents), self.assertRaises(ValueError):
                train.build_corpus(intents, {})

    def test_duplicate_values_do_not_consume_template_cap(self):
        stats = {}
        texts, labels = train.build_corpus(
            [{"name": "reminder", "templates": ["提醒我{task}"]}],
            {"task": {"values": ["喝水"] * 40 + ["吃药", "开会"]}}, stats=stats,
        )
        by_text = dict(zip(texts, labels))
        for task in ("喝水", "吃药", "开会"):
            self.assertEqual(by_text[f"提醒我{task}"], "reminder")
        self.assertEqual(stats["slot_value_duplicates_removed"], 39)
        self.assertEqual(stats["expansion_templates_limited"], 0)

    def test_repeated_slot_uses_one_combination_dimension(self):
        stats = train.Counter()
        expanded = train.expand_template(
            "{room}和{room}", {"room": {"values": ["客厅", "卧室"]}}, 40, stats=stats,
        )
        self.assertEqual(expanded, ["客厅和客厅", "卧室和卧室"])
        self.assertEqual(stats["expansion_combinations_scanned"], 2)

    def test_sentence_deduplication_precedes_output_cap(self):
        stats = train.Counter()
        expanded = train.expand_template(
            "提醒我{task}", {"task": {"values": ["喝水", "喝水 ", "吃药"]}},
            2, stats=stats,
        )
        self.assertEqual(expanded, ["提醒我喝水", "提醒我吃药"])
        self.assertEqual(stats["expansion_duplicate_sentences_removed"], 1)

    def test_scan_budget_bounds_combinatorial_expansion(self):
        stats = train.Counter()
        slots = {name: {"values": [str(i) for i in range(100)]} for name in ("a", "b", "c")}
        with patch.object(train, "TEMPLATE_SCAN_LIMIT", 3):
            expanded = train.expand_template("{a}/{b}/{c}", slots, 40, stats=stats)
        self.assertEqual(len(expanded), 3)
        self.assertEqual(stats["expansion_combinations_scanned"], 3)
        self.assertEqual(stats["expansion_templates_limited"], 1)

    def test_conflict_scope_is_explicitly_bounded(self):
        stats = {}
        texts, labels = train.build_corpus([
            {"name": "first", "templates": ["提醒我{task}"]},
            {"name": "second", "templates": ["提醒我第41项"]},
        ], {"task": {"values": [f"第{i}项" for i in range(1, 42)]}}, stats=stats)
        # 第41项未在first候选中展开，不宣称对它做过全组合冲突检查。
        self.assertEqual(dict(zip(texts, labels))["提醒我第41项"], "second")
        self.assertEqual(stats["expansion_templates_limited"], 1)
        self.assertEqual(stats["template_unique_cap"], 40)
        self.assertIn("bounded_expanded_candidates", stats["conflict_check_scope"])

    def test_rejects_missing_empty_and_nonlist_templates(self):
        for intent in (
            {"name": "empty"}, {"name": "empty", "templates": []},
            {"name": "empty", "templates": "不是列表"},
            {"name": "empty", "templates": [None]},
        ):
            with self.subTest(intent=intent), self.assertRaises(ValueError):
                train.build_corpus([intent], {})

    def test_rejects_nonpositive_per_intent_limit(self):
        for limit in (0, -1):
            with patch.object(train, "PER_INTENT", limit), self.assertRaisesRegex(ValueError, "PER_INTENT"):
                train.build_corpus([{"name": "skill", "templates": ["正句"]}], {})

    def test_stratified_split_is_deterministic_disjoint_and_complete(self):
        labels = ["small"] * 2 + ["large"] * 30 + ["other"] * 15
        label_list = ["small", "large", "other"]
        first = train.stratified_split_indices(labels, label_list)
        self.assertEqual(first, train.stratified_split_indices(labels, label_list))
        training, testing = first
        self.assertFalse(set(training) & set(testing))
        self.assertEqual(sorted(training + testing), list(range(len(labels))))
        for indices in first:
            self.assertEqual({labels[i] for i in indices}, set(label_list))
        self.assertEqual(sum(labels[i] == "small" for i in training), 1)
        self.assertEqual(sum(labels[i] == "small" for i in testing), 1)

    def test_stratified_split_rejects_missing_or_singleton_class(self):
        for labels in (["skill", "other", "other"], ["other", "other"]):
            with self.subTest(labels=labels), self.assertRaisesRegex(ValueError, "skill.*at least 2 unique"):
                train.stratified_split_indices(labels, ["skill", "other"])

    def test_stratified_split_rejects_invalid_parameters(self):
        for fraction in (0, 1, -0.1):
            with self.subTest(fraction=fraction), self.assertRaises(ValueError):
                train.stratified_split_indices(["skill"] * 2, ["skill"], fraction)
        with self.assertRaisesRegex(ValueError, "unexpected corpus label"):
            train.stratified_split_indices(["unknown"], ["skill"])
        with self.assertRaisesRegex(ValueError, "at least one label"):
            train.stratified_split_indices([], [])

    def test_new_no_slot_skill_with_64_templates_can_be_split(self):
        texts, labels = train.build_corpus([
            {"name": "help", "templates": [f"帮我了解主题{i}" for i in range(64)]},
        ], {})
        training, testing = train.stratified_split_indices(labels, ["help", "other"])
        self.assert_consistent(texts, labels)
        self.assertEqual(labels.count("help"), 64)
        self.assertEqual(sum(labels[i] == "help" for i in training), 58)
        self.assertEqual(sum(labels[i] == "help" for i in testing), 6)

    def test_main_rejects_empty_configuration_before_ml_import(self):
        with patch.object(train, "load_input", return_value=([], {})):
            with self.assertRaisesRegex(ValueError, "at least one intent"):
                train.main()

    def test_main_wires_stratified_indices_to_trainer(self):
        captured = {}

        class StopBeforeTraining(Exception):
            pass

        class FakeDataset:
            @classmethod
            def from_dict(cls, data):
                captured["data"] = data
                return cls()

            def map(self, tokenize, batched):
                self_check = tokenize({"text": ["测试句"]})
                captured["tokenized"] = self_check
                return self

            def select(self, indices):
                return list(indices)

        def trainer(**kwargs):
            captured.update(kwargs)
            raise StopBeforeTraining()

        tokenizer = lambda texts, **kwargs: {"input_ids": [[1]]}
        tokenizer_calls = []

        def load_tokenizer(name, **kwargs):
            tokenizer_calls.append((name, kwargs))
            return tokenizer

        modules = {
            "datasets": SimpleNamespace(Dataset=FakeDataset),
            "transformers": SimpleNamespace(
                AutoTokenizer=SimpleNamespace(from_pretrained=load_tokenizer),
                AutoModelForSequenceClassification=SimpleNamespace(
                    from_pretrained=lambda name, **kwargs: object()
                ),
                DataCollatorWithPadding=lambda tokenizer: object(),
                TrainingArguments=lambda **kwargs: kwargs,
                Trainer=trainer,
            ),
        }
        config = ([{"name": "help", "templates": [f"帮助主题{i}" for i in range(64)]}], {})
        with patch.dict(sys.modules, modules), patch.object(train, "load_input", return_value=config):
            with patch("builtins.print"), self.assertRaises(StopBeforeTraining):
                train.main()
        training, testing = captured["train_dataset"], captured["eval_dataset"]
        self.assertFalse(set(training) & set(testing))
        self.assertEqual(sorted(training + testing), list(range(len(captured["data"]["text"]))))
        for indices in (training, testing):
            self.assertEqual({captured["data"]["label"][i] for i in indices}, {0, 1})
        self.assertEqual(captured["tokenized"], {"input_ids": [[1]]})
        # 底座 profile 决定 hub 与超参：分词器必须指名 hub，并要求 fast 分词器。
        self.assertEqual(tokenizer_calls, [(train.HUB_NAME, {"use_fast": True})])
        self.assertEqual(captured["args"]["learning_rate"], train.LEARNING_RATE)
        self.assertEqual(captured["args"]["num_train_epochs"], train.EPOCHS)

    def test_import_and_corpus_generation_without_ml_dependencies(self):
        # 子进程主动阻断 ML 包，保证环境即使已安装它们也不掩盖导入回归。
        script = '''
import importlib.abc
import sys
class BlockML(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if fullname.split('.')[0] in {'torch', 'datasets', 'transformers', 'optimum', 'onnxruntime'}:
            raise AssertionError('unexpected ML import: ' + fullname)
sys.meta_path.insert(0, BlockML())
import train
texts, labels = train.build_corpus([{'name': 'test', 'templates': ['测试句']}], {})
assert dict(zip(texts, labels))['测试句'] == 'test'
train.load_input = lambda: ([{'name': 'single', 'templates': ['单句']}], {})
try:
    train.main()
except ValueError as error:
    assert 'at least 2 unique' in str(error)
else:
    raise AssertionError('singleton class was not rejected before ML imports')
'''
        result = subprocess.run(
            [sys.executable, "-B", "-c", script], cwd=Path(__file__).parent,
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


class DefaultSkillCorpusTests(unittest.TestCase):
    """训练只消费配置数据；动作步骤不能进入正句，也不能执行任何能力。"""

    @staticmethod
    def default_catalog():
        path = Path(__file__).resolve().parents[2] / "server/src/plugins/nlu/defaults.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        # 服务端共享 catalog 的命令源由 Rust 回归验证；这里验证无槽位命令与
        # 同一份默认技能 JSON 能共同生成不重叠语料，而不是第二套默认定义。
        intents = [{"name": "end_session", "templates": ["退下", "闭嘴", "关闭"]}]
        return intents + data["intents"], {s["name"]: s for s in data["slots"]}

    def test_initial_catalog_includes_commands_and_all_default_skill_labels(self):
        intents, slots = self.default_catalog()
        texts, labels = train.build_corpus(intents, slots)
        expected = [intent["name"] for intent in intents] + ["other"]
        self.assertTrue({"end_session", "reminder_countdown", "reminder_schedule",
                         "reminder_repeat", "reminder_daily", "reminder_workdays",
                         "reminder_weekly", "reminder_monthly", "reminder_yearly",
                         "reminder_hourly", "reminder_list", "reminder_cancel",
                         "other"}.issubset(labels))
        self.assertEqual(len(texts), len(set(texts)))
        by_text = dict(zip(texts, labels))
        for phrase in intents[0]["templates"]:
            self.assertEqual(by_text[phrase], "end_session")
        training, testing = train.stratified_split_indices(labels, expected)
        self.assertFalse(set(training) & set(testing))
        for indices in (training, testing):
            self.assertEqual({labels[i] for i in indices}, set(expected))

    def test_interval_and_daily_open_slots_have_meaningful_training_values(self):
        self.assertNotEqual(train.fill_values("interval", {}), train.GENERIC_FILLER)
        self.assertNotEqual(train.fill_values("daily_time", {}), train.GENERIC_FILLER)
        for value in train.fill_values("daily_time", {}):
            self.assertFalse(any(marker in value for marker in ("明天", "后天", "每周")))
        intents, slots = self.default_catalog()
        daily = next(intent for intent in intents if intent["name"] == "reminder_daily")
        expanded = [sentence for template in daily["templates"]
                    for sentence in train.expand_template(template, slots, 40)]
        self.assertTrue(expanded)
        for sentence in expanded:
            self.assertNotIn("明天", sentence)
            self.assertNotIn("后天", sentence)
            self.assertNotIn("这件事提醒", sentence)

    def test_calendar_expansion_covers_multiple_months_and_days_within_the_same_budget(self):
        _, slots = self.default_catalog()
        expanded = train.expand_template(
            "每年{month}月{day}日{daily_time}提醒我{task}", slots, 40)
        self.assertLessEqual(len(expanded), 40)
        for month in train.fill_values("month", slots):
            self.assertTrue(any(text.startswith(f"每年{month}月") for text in expanded), month)
        for day in train.fill_values("day", slots):
            self.assertTrue(any(f"月{day}日" in text for text in expanded), day)
        for name in ("weekday", "day", "month", "minute"):
            self.assertNotEqual(train.fill_values(name, slots), train.GENERIC_FILLER)

    def test_unseparated_numeric_slots_are_not_synthesized_as_executable_positives(self):
        import re
        _, slots = self.default_catalog()
        stats = train.Counter()
        expanded = train.expand_template(
            "每周{weekday}{daily_time}提醒我{task}", slots, 40, stats=stats)
        self.assertTrue(expanded)
        self.assertGreater(stats["expansion_ambiguous_adjacent_values_skipped"], 0)
        for text in expanded:
            self.assertIsNone(re.match(r"每周[1-7][0-9]", text), text)
        with_separator = train.expand_template(
            "每周{weekday}的{daily_time}提醒我{task}", slots, 40)
        self.assertTrue(any(re.match(r"每周[1-7]的", text) for text in with_separator))

    def test_all_clock_actions_use_static_capabilities_and_real_result_receipts(self):
        intents, _ = self.default_catalog()
        for intent in intents[1:]:
            with self.subTest(intent=intent["name"]):
                action = intent["action"]
                self.assertEqual(action["kind"], "plan")
                self.assertEqual(action["reply"], "")
                steps = action["plan"]["steps"]
                self.assertIn(steps[0]["api"], ("tasks.create", "tasks.list", "tasks.cancel"))
                self.assertEqual(steps[-1], {"api": "replies.say", "args": {"text": "{{steps.0.speech}}"}})
                if steps[0]["api"] == "tasks.create":
                    self.assertEqual(steps[0]["args"]["timezone"], "Asia/Shanghai")

    def test_configured_command_phrases_are_not_replaced_with_builtin_keywords(self):
        phrases = ["暂停对话", "结束本次聆听"]
        texts, labels = train.build_corpus(
            [{"name": "end_session", "templates": phrases}], {})
        by_text = dict(zip(texts, labels))
        self.assertEqual({text for text, label in by_text.items() if label == "end_session"},
                         set(phrases))
        self.assertNotIn("退下", by_text)


class BaseModelTests(unittest.TestCase):
    """底座是**单变量维度**：只允许两臂，名字错就失败，不猜 hub id。"""

    def test_only_two_supported_bases_and_shipped_default(self):
        self.assertEqual(sorted(train.BASES), ["minirbt-h288", "rbt3"])
        # 首项即缺省臂：服务端 DEFAULT_BASE_MODEL 与前端 view 初值必须同批对齐，
        # 顺序本身也是契约（有 Rust 单测从这份源码里读首项）。
        self.assertEqual(list(train.BASES)[0], "minirbt-h288")
        if "MODEL_NAME" not in os.environ:
            # 外部环境显式指定底座时不代表默认臂，跳过默认值断言。
            self.assertEqual(train.BASE_NAME, "minirbt-h288")
            self.assertEqual(train.HUB_NAME, "hfl/minirbt-h288")
        for name, profile in train.BASES.items():
            with self.subTest(base=name):
                self.assertEqual(set(profile), {"hub", "lr", "epochs"})
                self.assertIn("/", profile["hub"], "hub 必须是 org/name 形式")
                self.assertGreater(profile["lr"], 0.0)
                self.assertGreater(profile["epochs"], 0.0)

    def test_profiles_keep_distinct_hyperparameters(self):
        minirbt = train.resolve_base("minirbt-h288")
        self.assertEqual(minirbt, {"hub": "hfl/minirbt-h288", "lr": 1e-4, "epochs": 10.0})
        self.assertEqual(
            train.resolve_base("rbt3"), {"hub": "hfl/rbt3", "lr": 5e-5, "epochs": 6.0}
        )

    def test_unknown_base_is_rejected_with_allowed_values(self):
        # 历史语义（直接传 hub id）与将来可能的新底座都不能被静默接受。
        for name in ("hfl/rbt3", "rbt4-h312", "", " RBT3 "):
            with self.subTest(base=name), self.assertRaisesRegex(ValueError, "allowed"):
                train.resolve_base(name)
        with self.assertRaisesRegex(ValueError, "minirbt-h288"):
            train.resolve_base("rbt4-h312")

    def test_unknown_base_env_fails_before_any_ml_import(self):
        # 导入期失败：容器在下载权重之前退出，退出码非零且原因可读。
        result = subprocess.run(
            [sys.executable, "-B", "-c", "import train"],
            cwd=Path(__file__).parent,
            env=dict(os.environ, MODEL_NAME="rbt4-h312"),
            capture_output=True, text=True, check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unknown base model", result.stderr)
        self.assertIn("allowed", result.stderr)


class SmokeCaseTests(unittest.TestCase):
    """冒烟自报：只送**真实展开句**的文本，服务端拿它独立重跑判定。"""

    def corpus(self):
        intents = [
            {"name": "reminder_daily", "templates": ["每天{daily_time}提醒我{task}"]},
            {"name": "help_guide", "templates": ["告诉我怎么使用", "介绍一下功能"]},
        ]
        slots = {
            "daily_time": {"values": ["八点", "晚上九点半"]},
            "task": {"values": ["喝水", "开会"]},
        }
        return train.build_corpus(intents, slots)

    def test_one_case_per_label_plus_exactly_three_negatives_without_braces(self):
        texts, labels = self.corpus()
        smoke = train.smoke_cases(texts, labels)
        self.assertFalse(smoke["truncated"], "正常体量的语料不该触发裁剪")
        cases = smoke["cases"]
        by_expected = {}
        for case in cases:
            by_expected.setdefault(case["expected"], []).append(case["text"])
        self.assertEqual(sorted(by_expected), ["help_guide", "other", "reminder_daily"])
        self.assertEqual(len(by_expected["reminder_daily"]), 1)
        self.assertEqual(len(by_expected["help_guide"]), 1)
        # 负例**恒为 3 条**：服务端要求 3/3 全拒识，裁掉就会把"数据不够"报成"模型不好"。
        self.assertEqual(len(by_expected["other"]), 3)
        known = set(labels)
        for case in cases:
            self.assertIn(case["expected"], known)
            self.assertNotIn("{", case["text"])
            self.assertNotIn("}", case["text"])
            self.assertTrue(case["text"].strip())
        # 文本必须来自真实语料，不能是模板原文。
        self.assertIn(cases[0]["text"], set(texts))

    def test_output_is_deterministic_and_bounded_to_the_log_tail_budget(self):
        texts, labels = self.corpus()
        first = train.smoke_cases(texts, labels)
        self.assertEqual(first, train.smoke_cases(texts, labels))
        # 服务端只保留日志尾 8192 字节：整行必须压在 2KB 以内。
        line = json.dumps(first, ensure_ascii=False, separators=(",", ":"))
        self.assertLessEqual(len(line.encode()), 2000)

    def test_oversized_catalog_trims_positives_only_and_flags_truncation(self):
        intents = [
            {"name": f"skill_{i:02d}", "templates": [f"请帮我处理第{i}类请求并给出结果"]}
            for i in range(40)
        ]
        texts, labels = train.build_corpus(intents, {})
        # 故意给一个很小的上限，验证"只裁正例、负例恒 3"的裁剪路径。
        smoke = train.smoke_cases(texts, labels, limit=600)
        positives = [c for c in smoke["cases"] if c["expected"] != "other"]
        negatives = [c for c in smoke["cases"] if c["expected"] == "other"]
        self.assertGreaterEqual(len(positives), 1)
        self.assertEqual(len(negatives), 3, "负例永远不裁")
        self.assertTrue(smoke["truncated"], "裁过正例必须如实标记")
        line = json.dumps(smoke, ensure_ascii=False, separators=(",", ":"))
        self.assertLessEqual(len(line.encode()), 600)

    def test_smoke_line_sits_right_after_done_and_before_the_sample_probe(self):
        # 服务端只在日志尾 8192 字节里**从后向前**找这一行：太早会被 trainer 输出挤出窗口，
        # 太晚会被 STREAM 缓冲/截断波及。约定位置 = done 之后、sample 探针之前。
        source = Path(train.__file__).read_text(encoding="utf-8")
        smoke_at = source.index("[train] smoke=")
        self.assertGreater(smoke_at, source.index("[train] done"))
        self.assertLess(smoke_at, source.index("[train] sample argmax="))

    def test_unexpanded_templates_are_never_emitted(self):
        # 万一语料里混进花括号句（模板没被展开），宁可少一条也不能把它当正例送出去：
        # 服务端会按 smoke_missing 失败关闭，白跑一次训练。
        smoke = train.smoke_cases(["提醒我{task}", "介绍功能"], ["x", "x"])
        self.assertEqual(
            smoke, {"cases": [{"text": "介绍功能", "expected": "x"}], "truncated": False}
        )


if __name__ == "__main__":
    unittest.main()
