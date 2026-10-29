#!/usr/bin/env python3
"""状态观察机制机械化改造（Kuikly `observable` → Compose snapshot state）。

## 这个脚本解决什么

T05 要把全仓 `client/shared/src` 下 4 百多处

    var greeting by scope.observable("我在")            // 值
    val rows: ObservableList<HealthRow> by scope.observableList()   // 列表

换成 Compose 快照机制

    var greeting by stateVar("我在")
    val rows: MutableList<HealthRow> = stateListOf()

`observable` 驱动的是 Kuikly DSL 的 attr 重算，GearUI / 自有 Compose 组件读不到它；
换成 `mutableStateOf` 之后，Kit 组件与自有组件共享同一套重组触发机制。字段名、声明类型
（仅列表的 `ObservableList<T>` → `MutableList<T>`）、默认值、方法签名一律不动。

手工改 4 百多处既慢又容易改错一个字符，故机械化。**脚本不做"猜"**：任何一行含
`observable` 却不匹配已知形态的，立即中止并打印该行，由人决定——宁可停下来，也不要
写出一个"看起来改完了"的仓库。

## 五个阶段

| 阶段 | 名称 | 动作 |
|---|---|---|
| S1 | collect | 扫描 5 个源集，按 8 种形态归类；出现未知形态 → 中止 |
| S2 | values | `by <recv>.observable<T?>(v)` → `by stateVar<T?>(v)`（含无接收者的裸调用） |
| S3 | lists | `by observableList<T>()` → `= stateListOf()`，声明类型 `ObservableList<T>` → `MutableList<T>` |
| S4 | imports | 三个 `reactive.*` 导入就地替换/删除；转换后没用到的补丁导入回删 |
| S5 | verify | 复扫：遗留原语必须为 0；打印 `stateVar` / `stateListOf` 计数 |

## 用法

    python3 scripts/codemod_compose_state.py --check     # 只体检，不写盘（CI / 护栏用）
    python3 scripts/codemod_compose_state.py             # 执行 S1→S5，逐文件改写
    python3 scripts/codemod_compose_state.py --dry-run    # 执行但不落盘，打印将要改的行数

退出码：0 成功；1 发现未知形态或校验失败（此时不会有任何文件被"改一半"，见下）。

## 原子性

先对**全部**文件跑 S1（只读），确认没有未知形态，才进入写盘阶段；写盘阶段逐文件
「读→改→写」，任一文件写失败即中止并打印已改文件清单（可用 `git diff` 回退）。
故最坏情况是"改了但没错"，不会产生语法半成品。
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

# --------------------------------------------------------------------------
# 常量
# --------------------------------------------------------------------------

#: 源集根目录（相对 client/）。新增源集必须在此登记，否则会漏改。
SOURCE_SETS = (
    "commonMain",
    "androidMain",
    "iosMain",
    "jsMain",
    "macosArm64Main",
)

#: 旧机制的三条导入（S4 要处理的就是这三种）
IMPORT_OBSERVABLE = "import com.tencent.kuikly.core.reactive.handler.observable"
IMPORT_OBSERVABLE_LIST = "import com.tencent.kuikly.core.reactive.handler.observableList"
IMPORT_OBSERVABLE_LIST_TYPE = "import com.tencent.kuikly.core.reactive.collection.ObservableList"

#: 新机制的导入
IMPORT_STATE_VAR = "import com.xiaozhi.admin.state.stateVar"
IMPORT_STATE_LIST_OF = "import com.xiaozhi.admin.state.stateListOf"

#: 修饰符（保持原位，不重排）
MODS = r"(?:(?:internal|private|public|protected)\s+)?"

#: Kotlin 类型文本：允许一层嵌套泛型（`Pair<String, String>`、`Map<String, List<Foo>>` 里
#: 本仓不会更深，先支持一层；出现更深的形态会被 S1 判为未知并按纪律中止）。
TYPE = r"(?:[^<>]|<[^<>]*>)+"

#: S3-a：`val rows: ObservableList<T> by <recv>.observableList()`（类型显式）
LIST_TYPED = re.compile(
    rf"^(?P<indent>\s*)(?P<mods>{MODS})(?P<kw>var|val)\s+(?P<name>\w+)\s*:\s*"
    rf"ObservableList\s*<(?P<type>{TYPE})>\s*by\s+(?:\w+\.)?observableList"
    rf"(?P<gen>\s*<{TYPE}>)?\s*\(\s*\)\s*$"
)

#: S3-b：`var options by observableList<Pair<String, String>>()`（类型推断）
LIST_INFERRED = re.compile(
    rf"^(?P<indent>\s*)(?P<mods>{MODS})(?P<kw>var|val)\s+(?P<name>\w+)\s+by\s+"
    rf"(?:\w+\.)?observableList\s*<(?P<type>{TYPE})>\s*\(\s*\)\s*$"
)

#: S2：`var x by <recv>.observable<G>(init)` / `by observable(init)`
VALUE = re.compile(
    rf"^(?P<indent>\s*)(?P<mods>{MODS})(?P<kw>var|val)\s+(?P<name>\w+)"
    rf"(?P<ty>\s*:\s*[^=]+?)?\s+by\s+(?:\w+\.)?observable"
    rf"(?P<gen>\s*<{TYPE}>)?\s*\((?P<init>.*)\)\s*$"
)

#: S3-c：残留的 `ObservableList<…>` 类型引用（函数参数、lambda 形参等）
LIST_TYPE_REF = re.compile(r"\bObservableList\s*<")

#: 旧机制任何痕迹（S1/S5 用来判断"这行到底该不该动"）
ANY_LEGACY = re.compile(r"\bobservable(List)?\b|ObservableList")


def repo_root() -> Path:
    """client/ 目录：脚本位于 client/scripts/ 下。"""
    return Path(__file__).resolve().parent.parent


def source_files() -> list[Path]:
    """按 SOURCE_SETS 收集 .kt（排序保证每次运行顺序一致）。"""
    root = repo_root() / "shared" / "src"
    files: list[Path] = []
    for src_set in SOURCE_SETS:
        base = root / src_set / "kotlin"
        if base.is_dir():
            files.extend(sorted(base.rglob("*.kt")))
    return files


@dataclass
class Plan:
    """一个文件的改写计划：行号 → 新行的全集，外加统计。"""

    path: Path
    lines: list[str]
    replacements: dict[int, str] = field(default_factory=dict)
    values: int = 0
    lists: int = 0
    type_refs: int = 0
    import_lines_removed: int = 0

    @property
    def changed(self) -> bool:
        return bool(self.replacements)

    def render(self) -> str:
        out = list(self.lines)
        for index, new_line in self.replacements.items():
            out[index] = new_line
        return "".join(out)


def is_comment(line: str) -> bool:
    """行首是注释 → 不参与机械改写（KDoc 里会引用旧 API 名，属正常说明）。"""
    stripped = line.strip()
    return stripped.startswith("//") or stripped.startswith("*") or stripped.startswith("/*")


def code_part(line: str) -> str:
    """去掉行尾行注释后的代码部分（`a // b` 只保留 `a `）。"""
    return line.split(" //")[0]


# --------------------------------------------------------------------------
# S1 · collect：分类 + 未知形态中止
# --------------------------------------------------------------------------


def scan(path: Path) -> tuple[Plan, list[tuple[int, str]]]:
    """扫描单个文件，返回（改写计划，未知行列表）。

    未识别形态不写进计划，交给上层决定是否中止——脚本不替人猜。
    """
    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    plan = Plan(path=path, lines=lines)
    unknown: list[tuple[int, str]] = []

    for index, raw in enumerate(lines):
        if is_comment(raw):
            continue
        code = code_part(raw).rstrip("\r\n")
        if not ANY_LEGACY.search(code):
            continue
        # 导入行交给 S4，这里不算"未知"
        if code.strip().startswith("import "):
            continue

        if matched := LIST_TYPED.match(code):
            type_text = matched.group("type").strip()
            new_line = (
                f"{matched.group('indent')}{matched.group('mods')}{matched.group('kw')} "
                f"{matched.group('name')}: MutableList<{type_text}> = stateListOf()\n"
            )
            plan.replacements[index] = new_line
            plan.lists += 1
            continue

        if matched := LIST_INFERRED.match(code):
            type_text = matched.group("type").strip()
            new_line = (
                f"{matched.group('indent')}{matched.group('mods')}{matched.group('kw')} "
                f"{matched.group('name')}: MutableList<{type_text}> = stateListOf()\n"
            )
            plan.replacements[index] = new_line
            plan.lists += 1
            continue

        if matched := VALUE.match(code):
            generic = (matched.group("gen") or "").replace(" ", "")
            declared = matched.group("ty") or ""
            new_line = (
                f"{matched.group('indent')}{matched.group('mods')}{matched.group('kw')} "
                f"{matched.group('name')}{declared} by stateVar{generic}({matched.group('init')})\n"
            )
            plan.replacements[index] = new_line
            plan.values += 1
            continue

        # 残余的 ObservableList 类型引用（函数参数、lambda 形参）：就地换成 MutableList
        if LIST_TYPE_REF.search(code):
            plan.replacements[index] = LIST_TYPE_REF.sub("MutableList<", raw)
            plan.type_refs += 1
            continue

        unknown.append((index + 1, raw.rstrip()))

    return plan, unknown


# --------------------------------------------------------------------------
# S4 · imports
# --------------------------------------------------------------------------


def rewrite_imports(plan: Plan) -> None:
    """三个旧导入就地替换；转换后没人用的新导入回删（避免留下未使用导入告警）。"""
    text_after = plan.render()
    code_after = "\n".join(
        "" if is_comment(line) else code_part(line) for line in text_after.splitlines()
    )
    needs_state_var = bool(re.search(r"\bstateVar\s*(<[^<>]*>)?\s*\(", code_after))
    needs_state_list = bool(re.search(r"\bstateListOf\s*(<[^<>]*>)?\s*\(", code_after))

    for index, raw in enumerate(plan.lines):
        stripped = raw.strip()
        if stripped == IMPORT_OBSERVABLE:
            if needs_state_var:
                plan.replacements[index] = raw.replace(IMPORT_OBSERVABLE, IMPORT_STATE_VAR)
            else:
                plan.replacements[index] = ""
                plan.import_lines_removed += 1
        elif stripped == IMPORT_OBSERVABLE_LIST:
            if needs_state_list:
                plan.replacements[index] = raw.replace(IMPORT_OBSERVABLE_LIST, IMPORT_STATE_LIST_OF)
            else:
                plan.replacements[index] = ""
                plan.import_lines_removed += 1
        elif stripped == IMPORT_OBSERVABLE_LIST_TYPE:
            # 类型已全部迁到 MutableList（kotlin.collections，无需导入）
            plan.replacements[index] = ""
            plan.import_lines_removed += 1


# --------------------------------------------------------------------------
# S5 · verify
# --------------------------------------------------------------------------

LEGACY_IMPORT = re.compile(r"import\s+com\.tencent\.kuikly\.core\.reactive\.")
LEGACY_VALUE = re.compile(r"\bby\s+(?:\w+\.)?observable\s*[(<]")
LEGACY_LIST_CALL = re.compile(r"\bby\s+(?:\w+\.)?observableList\s*[(<]")
LEGACY_TYPE = re.compile(r"\bObservableList\s*<")
STATE_VAR_CALL = re.compile(r"\bstateVar\s*(<[^<>]*>)?\s*\(")
STATE_LIST_CALL = re.compile(r"\bstateListOf\s*(<[^<>]*>)?\s*\(")


@dataclass
class Counts:
    """全仓计数。定义文件单独剔除，否则 `fun stateVar(` 声明会被算成一次调用。"""

    values: int = 0
    lists: int = 0
    legacy: list[str] = field(default_factory=list)

    def summary(self) -> str:
        return (
            f"stateVar={self.values} 次调用，stateListOf={self.lists} 次调用，"
            f"遗留原语={len(self.legacy)} 处"
        )


def count_repo() -> Counts:
    counts = Counts()
    definition = repo_root() / "shared/src/commonMain/kotlin/com/xiaozhi/admin/state/StateVar.kt"
    for path in source_files():
        is_definition = path.resolve() == definition.resolve()
        for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if is_comment(raw):
                continue
            code = code_part(raw)
            if not is_definition:
                counts.values += len(STATE_VAR_CALL.findall(code))
                counts.lists += len(STATE_LIST_CALL.findall(code))
            if (
                LEGACY_IMPORT.search(code)
                or LEGACY_VALUE.search(code)
                or LEGACY_LIST_CALL.search(code)
                or LEGACY_TYPE.search(code)
            ):
                rel = path.relative_to(repo_root())
                counts.legacy.append(f"{rel}:{lineno}: {raw.strip()}")
    return counts


# --------------------------------------------------------------------------
# 主流程
# --------------------------------------------------------------------------


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Kuikly observable → Compose snapshot state 机械化改造")
    parser.add_argument("--check", action="store_true", help="只体检不写盘；有遗留原语则退出码 1")
    parser.add_argument("--dry-run", action="store_true", help="执行 S1→S4 但不落盘")
    args = parser.parse_args(argv)

    if args.check:
        counts = count_repo()
        print(f"[S5] {counts.summary()}")
        for line in counts.legacy[:40]:
            print(f"  遗留: {line}")
        return 1 if counts.legacy else 0

    # ---- S1 · collect（全部文件先只读扫一遍，确认无未知形态）----
    plans: list[Plan] = []
    all_unknown: list[str] = []
    for path in source_files():
        plan, unknown = scan(path)
        plans.append(plan)
        for lineno, text in unknown:
            rel = path.relative_to(repo_root())
            all_unknown.append(f"{rel}:{lineno}: {text.strip()}")

    if all_unknown:
        print("[S1] 发现无法识别的形态，已中止（未写任何文件）：")
        for line in all_unknown:
            print(f"  {line}")
        return 1

    planned_values = sum(p.values for p in plans)
    planned_lists = sum(p.lists for p in plans)
    planned_types = sum(p.type_refs for p in plans)
    print(f"[S1] 计划改写：值字段 {planned_values}，列表字段 {planned_lists}，类型引用 {planned_types}")

    # ---- S2/S3 · 字段与方法签名（计划里已完成，这里只是阶段日志）----
    print(f"[S2] values: {planned_values} 处 by observable(...) → by stateVar(...)")
    print(f"[S3] lists : {planned_lists} 处 observableList() → stateListOf()，"
          f"类型引用 {planned_types} 处 → MutableList")

    # ---- S4 · imports ----（旧导入就地替换成 state/ 包的导入，不重排其它 import）
    for plan in plans:
        rewrite_imports(plan)
    removed = sum(p.import_lines_removed for p in plans)
    print(f"[S4] imports: 删除废弃导入 {removed} 行")

    changed = [p for p in plans if p.changed]
    print(f"[S4] 涉及文件 {len(changed)} 个")

    if args.dry_run:
        for plan in changed:
            rel = plan.path.relative_to(repo_root())
            print(f"  (dry-run) {rel}: 值 {plan.values} / 列表 {plan.lists} / 类型 {plan.type_refs}")
        return 0

    written: list[Path] = []
    for plan in changed:
        rendered = plan.render()
        try:
            plan.path.write_text(rendered, encoding="utf-8")
        except OSError as error:  # 写失败 → 打印已改文件，交给 git diff 回退
            print(f"[!] 写盘失败：{plan.path}（{error}）")
            print("[!] 已写入的文件：")
            for path in written:
                print(f"  {path.relative_to(repo_root())}")
            return 1
        written.append(plan.path)

    # ---- S5 · verify ----
    counts = count_repo()
    print(f"[S5] {counts.summary()}")
    for line in counts.legacy[:40]:
        print(f"  遗留: {line}")
    if counts.legacy:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
