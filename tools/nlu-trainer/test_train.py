"""语料生成回归：只用 Python 标准库，不下载模型、不导入 torch。"""
import importlib.util
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
        modules = {
            "datasets": SimpleNamespace(Dataset=FakeDataset),
            "transformers": SimpleNamespace(
                AutoTokenizer=SimpleNamespace(from_pretrained=lambda name: tokenizer),
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


if __name__ == "__main__":
    unittest.main()
