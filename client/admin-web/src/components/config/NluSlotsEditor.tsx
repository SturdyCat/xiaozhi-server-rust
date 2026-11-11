/**
 * 槽位定义编辑器（W7 / Config C4）。
 *
 * 数据模型对齐服务端 `plugins/nlu/mod.rs:26-33`（`SlotList` = `{name, values,
 * optional}`，无扩展字段）与旧端 `ConfigNluState.kt:60-64,147-157`：
 * - 空 values = 开放槽位（任意短语都接受，不再校验）；
 * - `optional` = 允许缺省（模板里可以不说这个槽位，缺省时以空值执行）。
 *
 * **整段写回**（已定决策）：每次改动 `setField("nlu.slots", newWholeArray)`，
 * 与 intents 编辑器同口径；**禁止**下标路径、**禁止**整段 `nlu` 对象回写。
 *
 * 本地副本同步：与 `NluIntentsEditor` 同机制——effect 监听 draft 中 `nlu.slots`
 * 的引用变化，内容与本地副本构建的 wire 不一致时才重置（覆盖 409 后重新加载、
 * 保存成功回读、丢弃草稿三种外部重建场景；自己 commit 造成的变化不打断编辑）。
 */

import { useEffect, useRef, useState, type ReactElement } from "react";
import { ChevronDown, ChevronRight, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import type { JsonObject, JsonValue } from "@/lib/configApi";
import { stringListOf } from "@/components/form/StringListField";

/** 单条槽位的编辑态；`SlotList` 只有三个字段，无扩展字段需保留。 */
interface SlotEdit {
  name: string;
  values: string[];
  optional: boolean;
}

/** 判断是否为普通 JSON 对象（数组与 null 不算）。 */
function isJsonObject(value: JsonValue | undefined): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function normalizeSlot(source: JsonObject): SlotEdit {
  return {
    name: typeof source["name"] === "string" ? source["name"] : "",
    values: stringListOf(source["values"]),
    optional: source["optional"] === true,
  };
}

function normalizeSlots(value: JsonValue | undefined): SlotEdit[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const items: SlotEdit[] = [];
  for (const raw of value) {
    if (isJsonObject(raw)) {
      items.push(normalizeSlot(raw));
    }
  }
  return items;
}

/** `SlotList` 固定三字段，直接重建（无扩展字段）。 */
function slotToWire(edit: SlotEdit): JsonObject {
  return {
    name: edit.name.trim(),
    values: edit.values,
    optional: edit.optional,
  };
}

/** JSON 深比较（两侧都由同一条重建链构建，键序一致）。 */
function jsonEqual(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** 换行文本 → 字符串数组（每行 trim、丢空行）。 */
function parseLines(raw: string): string[] {
  return raw
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

function SlotBlock({
  edit,
  expanded,
  onToggle,
  onChange,
  onRemove,
}: {
  edit: SlotEdit;
  expanded: boolean;
  onToggle: () => void;
  onChange: (next: SlotEdit) => void;
  onRemove: () => void;
}): ReactElement {
  const update = (patch: Partial<SlotEdit>): void =>
    onChange({ ...edit, ...patch });

  return (
    <div className="rounded-md border border-border bg-muted/30">
      <div className="flex flex-wrap items-center gap-2 px-3 py-2">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 px-1"
          onClick={onToggle}
          aria-expanded={expanded}
        >
          {expanded ? (
            <ChevronDown aria-hidden="true" className="size-4" />
          ) : (
            <ChevronRight aria-hidden="true" className="size-4" />
          )}
        </Button>
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {"{"}
          {edit.name.trim() === "" ? "未命名槽位" : edit.name.trim()}
          {"}"}
        </span>
        <Button type="button" variant="outline" size="sm" onClick={onRemove}>
          <Trash2 aria-hidden="true" className="size-4" />
          删除
        </Button>
      </div>
      {expanded ? (
        <div className="divide-y divide-border px-3 pb-3">
          <div className="grid gap-1.5 py-2">
            <Label className="text-sm font-medium">槽位名</Label>
            <Input
              className="max-w-xs"
              value={edit.name}
              placeholder="room"
              onChange={(event) => update({ name: event.target.value })}
            />
            <p className="text-xs text-muted-foreground">
              必须与模板里的 {"{占位名}"} 完全一致（区分大小写）。改名后不会自动同步模板，需自行修改训练例句。
            </p>
          </div>
          <div className="grid gap-1.5 py-2">
            <Label className="text-sm font-medium">封闭值表（每行一个）</Label>
            <Textarea
              className="max-w-2xl"
              rows={3}
              value={edit.values.join("\n")}
              placeholder={"卧室\n客厅\n书房"}
              onChange={(event) => update({ values: parseLines(event.target.value) })}
            />
            <p className="text-sm font-medium">共 {edit.values.length} 条</p>
            <p className="text-xs text-muted-foreground">
              列出这个槽位所有合法取值，用于校验与抽取；留空表示开放槽位（任意短语都接受，不再校验）。
            </p>
          </div>
          <div className="flex items-center gap-3 py-2">
            <Switch
              id={`nlu-slot-optional-${edit.name}`}
              checked={edit.optional}
              onCheckedChange={(next) => update({ optional: next })}
            />
            <div className="grid gap-1">
              <Label htmlFor={`nlu-slot-optional-${edit.name}`} className="text-sm font-medium">
                允许缺省
              </Label>
              <p className="text-xs text-muted-foreground">
                打开后模板里可以不说这个槽位（如「开灯」省略房间），缺省时以空值执行；关闭则该槽位必须出现。
              </p>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function NluSlotsEditor(): ReactElement {
  const { getField, setField } = useConfigDraft();
  // draft 中 nlu.slots 的当前引用：外部重建时引用变化。
  const draftSlots = getField("nlu.slots");
  const [items, setItems] = useState<SlotEdit[]>(() => normalizeSlots(draftSlots));
  const [expandedIndex, setExpandedIndex] = useState<number | null>(null);
  const itemsRef = useRef<SlotEdit[]>(items);
  itemsRef.current = items;

  useEffect(() => {
    // 外部重建检测（409 后重新加载 / 保存成功回读 / 丢弃草稿）：内容不一致才重置。
    const wireFromLocal = itemsRef.current.map(slotToWire);
    if (!jsonEqual(draftSlots, wireFromLocal)) {
      setItems(normalizeSlots(draftSlots));
    }
  }, [draftSlots]);

  const commit = (next: SlotEdit[]): void => {
    setItems(next);
    // 整段写回：数组作为叶子值，绝不写下标路径。
    setField("nlu.slots", next.map(slotToWire));
  };

  const updateAt = (index: number, next: SlotEdit): void => {
    commit(itemsRef.current.map((item, i) => (i === index ? next : item)));
  };

  const removeAt = (index: number): void => {
    commit(itemsRef.current.filter((_, i) => i !== index));
    if (expandedIndex === index) {
      setExpandedIndex(null);
    }
  };

  const addSlot = (): void => {
    const next = [...itemsRef.current, { name: "", values: [], optional: false }];
    setExpandedIndex(next.length - 1);
    commit(next);
  };

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <p className="min-w-0 flex-1 text-sm font-medium">
          槽位定义（{items.length} 条）
        </p>
        <Button type="button" variant="outline" size="sm" onClick={addSlot}>
          <Plus aria-hidden="true" className="size-4" />
          添加槽位
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        训练例句中的 {"{槽位}"} 在这里声明。只有模型已经接受同一技能后，才提取、校验这些参数；缺少必填参数时明确请求补充，不从分数、其他技能或大模型猜造参数。模板里用到的
        {"{槽位}"} 在这里命名并给封闭值表；不给值表 = 开放槽位。
      </p>
      {items.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          暂无槽位定义；无槽位技能无需添加槽位。
        </p>
      ) : (
        <div className="space-y-2">
          {items.map((edit, index) => (
            <SlotBlock
              key={`${edit.name}:${index}`}
              edit={edit}
              expanded={expandedIndex === index}
              onToggle={() =>
                setExpandedIndex((current) => (current === index ? null : index))
              }
              onChange={(next) => updateAt(index, next)}
              onRemove={() => removeAt(index)}
            />
          ))}
        </div>
      )}
    </div>
  );
}
