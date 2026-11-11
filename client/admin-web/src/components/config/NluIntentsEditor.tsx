/**
 * 技能（意图）条目编辑器（W7 / Config C4）。
 *
 * 数据模型对齐服务端 `plugins/nlu/mod.rs:148-169`（`IntentDef` / `ActionDef`）与
 * 旧端 `ConfigNluState.kt:13-58` 的 `IntentDraft`：
 * - 意图 = `{name, templates, action:{kind,reply,plan?}, domain}`；
 * - `kind` 是自由字符串：`"mcp:<工具名>"` 把设备工具名编码进 kind（defaults.json:151），
 *   动作类型四选 plan/reply/end_session/mcp，**未知 kind 保留原值**（对齐旧端
 *   `ConfigNluState.kt:142-144`「Never silently turn a future/unknown action into
 *   a reply action」），绝不纠正、绝不伪装成 reply；
 * - 扩展字段保留：写回从加载时的 `sourceIntent` / `sourceAction` 出发 spread
 *   （对齐旧端 `IntentDraft.load:24-25`「preserves extension fields」），只覆写
 *   已知键，plan 以外的未知 action 字段一律不丢。
 *
 * **整段写回**（已定决策）：每次改动 `setField("nlu.intents", newWholeArray)`，
 * 数组作为叶子值整体替换（服务端 `merge_patch` 非对象整体替换语义）；**禁止**
 * `nlu.intents.0.name` 这类下标路径（`writePath:149` 会把数组换成 `{}` 毁结构），
 * **禁止**照抄旧端 `putJson:113-128` 的整段 `nlu` 对象回写。
 *
 * 本地副本同步（最容易出错处）：副本从 draft 初始化；draft 被 reload / 保存回读 /
 * discard 重建后**必须重置副本**——用 effect 监听 draft 中 `nlu.intents` 的引用
 * 变化，内容与本地副本构建的 wire 不一致时才重置（自己 commit 造成的变化内容
 * 一致，不打断编辑）。三种重置场景：409 后重新加载、保存成功回读、丢弃草稿。
 */

import { useEffect, useRef, useState, type ReactElement } from "react";
import { ChevronDown, ChevronRight, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { FormNotice } from "@/components/form/FormNotice";
import { JsonTextField, nluPlanError } from "@/components/form/JsonTextField";
import { stringListOf } from "@/components/form/StringListField";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import type { JsonObject, JsonValue } from "@/lib/configApi";
import { NLU_DOMAINS } from "@/lib/nluApi";

/** 控制域选项（`ConfigNluState.kt:77-81` 同款文案）。 */
const DOMAIN_OPTIONS = [
  { value: "server", label: "服务器域（服务端执行）" },
  { value: "device", label: "设备域（固件执行）" },
  { value: "iot", label: "IoT 域（第三方设备 · tuya）" },
];

/** 动作类型基础选项（`ConfigNluState.kt:84-89` 同款）。 */
const BASE_ACTION_OPTIONS = [
  { value: "plan", label: "能力组合（条件与步骤）" },
  { value: "reply", label: "只应答（兼容已有技能）" },
  { value: "end_session", label: "结束聆听（兼容已有技能）" },
  { value: "mcp", label: "设备工具（兼容已有技能）" },
];

/** plan 模式默认 JSON（对齐旧端 `ConfigNluState.kt:21`）。 */
const DEFAULT_PLAN_TEXT = '{"conditions":[],"steps":[]}';

/** 判断是否为普通 JSON 对象（数组与 null 不算）。 */
function isJsonObject(value: JsonValue | undefined): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 单条意图的编辑态（含加载时快照，供扩展字段保留与改名检测）。 */
interface IntentEdit {
  name: string;
  domain: string;
  templates: string[];
  /** plan / reply / end_session / mcp，或加载时的未知原 kind（保留原值）。 */
  actionKind: string;
  /** `kind` 以 `mcp:` 开头时拆出的设备工具名。 */
  mcpTool: string;
  reply: string;
  /** plan JSON 的文本态：非法输入也保留显示，不被重置。 */
  planText: string;
  /** 最近一次通过守卫的 plan 对象；非法输入时保留上次合法值写回。 */
  lastValidPlan: JsonObject | null;
  /** 加载时的完整意图对象（扩展字段来源）。 */
  sourceIntent: JsonObject;
  /** 加载时的 action 对象（扩展字段来源）。 */
  sourceAction: JsonObject;
  /** 加载时的技能名（训练标签基线），用于改名强提示。 */
  sourceName: string;
}

/** 从草稿里的单条意图安全构建编辑态；未知控制域按服务端缺省规则归为 server。 */
function normalizeIntent(source: JsonObject): IntentEdit {
  const name = typeof source["name"] === "string" ? source["name"] : "";
  const domainRaw = typeof source["domain"] === "string" ? source["domain"] : "server";
  // 旧端 `ConfigNluState.kt:32-35`：未知/缺失 domain 缺省服务端域（写回非法值会 400）。
  const domain = (NLU_DOMAINS as readonly string[]).includes(domainRaw) ? domainRaw : "server";
  const sourceAction = isJsonObject(source["action"]) ? source["action"] : {};
  const kindRaw =
    typeof sourceAction["kind"] === "string" && sourceAction["kind"] !== ""
      ? sourceAction["kind"]
      : "reply";
  const isMcp = kindRaw === "mcp" || kindRaw.startsWith("mcp:");
  const planValue = sourceAction["plan"];
  const plan = isJsonObject(planValue) ? planValue : null;
  return {
    name,
    domain,
    templates: stringListOf(source["templates"]),
    actionKind: isMcp ? "mcp" : kindRaw,
    mcpTool: kindRaw.startsWith("mcp:") ? kindRaw.slice("mcp:".length) : "",
    reply: typeof sourceAction["reply"] === "string" ? sourceAction["reply"] : "",
    planText: plan !== null ? JSON.stringify(plan, null, 2) : DEFAULT_PLAN_TEXT,
    lastValidPlan: plan,
    sourceIntent: source,
    sourceAction,
    sourceName: name,
  };
}

function normalizeIntents(value: JsonValue | undefined): IntentEdit[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const items: IntentEdit[] = [];
  for (const raw of value) {
    // 非对象元素跳过（对齐旧端 `ConfigNluState.kt:140` 的 `?: continue`）。
    if (isJsonObject(raw)) {
      items.push(normalizeIntent(raw));
    }
  }
  return items;
}

/**
 * 把编辑态写回 wire 对象（对齐旧端 `IntentDraft.toJson` + `nluActionJson`）：
 * 从加载时快照 spread 保留扩展字段，只覆写已知键；kind !== plan 时删除 plan 键，
 * plan 非法时保留上次合法值（lastValidPlan），加载时无 plan 且从未输入合法值
 * 则不写 plan 键。
 */
function intentToWire(edit: IntentEdit): JsonObject {
  const action: JsonObject = { ...edit.sourceAction };
  const kind = edit.actionKind === "mcp" ? `mcp:${edit.mcpTool.trim()}` : edit.actionKind;
  action["kind"] = kind;
  action["reply"] = edit.reply;
  if (edit.actionKind !== "plan") {
    delete action["plan"];
  } else if (edit.lastValidPlan !== null) {
    action["plan"] = edit.lastValidPlan;
  }
  const intent: JsonObject = { ...edit.sourceIntent };
  intent["name"] = edit.name.trim();
  intent["domain"] = edit.domain;
  intent["templates"] = edit.templates;
  intent["action"] = action;
  return intent;
}

/** JSON 深比较（键序敏感即可：两侧都由同一条 spread 链构建，键序一致）。 */
function jsonEqual(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** 换行文本 → 字符串数组（每行 trim、丢空行），与 `StringListField` 同口径。 */
function parseLines(raw: string): string[] {
  return raw
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

/** 单条意图块：头部（名称 + 域 + 展开/删除）+ 展开详情。 */
function IntentBlock({
  edit,
  expanded,
  onToggle,
  onChange,
  onRemove,
}: {
  edit: IntentEdit;
  expanded: boolean;
  onToggle: () => void;
  onChange: (next: IntentEdit) => void;
  onRemove: () => void;
}): ReactElement {
  const planError = edit.actionKind === "plan" ? nluPlanError(edit.planText) : null;
  const renamed = edit.sourceName !== "" && edit.name.trim() !== edit.sourceName;
  const actionOptions = [...BASE_ACTION_OPTIONS];
  const knownAction = BASE_ACTION_OPTIONS.some((o) => o.value === edit.actionKind);
  if (!knownAction) {
    // 未知动作：动态补一个「保留原配置」选项，绝不静默改成 reply（旧端 :142-144 同款）。
    actionOptions.push({
      value: edit.actionKind,
      label: `其他动作（保留原配置：${edit.actionKind}）`,
    });
  }

  const update = (patch: Partial<IntentEdit>): void => onChange({ ...edit, ...patch });

  const updatePlanText = (raw: string): void => {
    const error = nluPlanError(raw);
    // 守卫通过才更新 lastValidPlan 并写回；失败保留上次合法值，文本态始终保留。
    update({
      planText: raw,
      lastValidPlan: error === null ? (JSON.parse(raw) as JsonObject) : edit.lastValidPlan,
    });
  };

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
          {edit.name.trim() === "" ? "(未命名技能)" : edit.name.trim()}
        </span>
        <Badge variant="outline">{edit.domain}</Badge>
        <Button type="button" variant="outline" size="sm" onClick={onRemove}>
          <Trash2 aria-hidden="true" className="size-4" />
          删除
        </Button>
      </div>
      {expanded ? (
        <div className="divide-y divide-border px-3 pb-3">
          <div className="grid gap-1.5 py-2">
            <p className="text-sm font-medium">技能名</p>
            <Input
              className="max-w-md font-mono"
              value={edit.name}
              placeholder="light_control"
              onChange={(event) => update({ name: event.target.value })}
            />
            <p className="text-xs text-muted-foreground">
              技能唯一标识 =
              模型训练标签（可用英文下划线）；同名技能会被服务端拒绝，不能使用保留标签
              other/end_session。
            </p>
            {renamed ? (
              <div className="pt-1">
                <FormNotice tone="warning">
                  技能改名会使旧模型标签失效：已改名「{edit.sourceName}」→「
                  {edit.name.trim() === "" ? "未命名" : edit.name.trim()}
                  」。技能名即模型训练标签，保存后必须重新训练并验收新模型，旧模型无法识别新名称；取消保存可保留旧标签。
                </FormNotice>
              </div>
            ) : null}
          </div>
          <div className="grid gap-1.5 py-2">
            <p className="text-sm font-medium">控制域</p>
            <select
              className="max-w-xs rounded-md border border-input bg-background px-3 py-2 text-sm"
              value={edit.domain}
              onChange={(event) => update({ domain: event.target.value })}
            >
              {DOMAIN_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
            <p className="text-xs text-muted-foreground">
              按执行者分域：设备域由固件工具执行（含断开对话），服务器域由服务端执行（提醒/天气类能力组合），IoT
              域由服务端执行（tuya
              第三方设备）。改域只影响目录分组展示，不影响训练标签；非法值保存会被服务端 400
              拒绝，界面已按服务端缺省规则归为 server。
            </p>
          </div>
          <div className="grid gap-1.5 py-2">
            <p className="text-sm font-medium">训练例句（每行一条）</p>
            <Textarea
              className="max-w-2xl"
              rows={4}
              value={edit.templates.join("\n")}
              placeholder="把{room}的灯{action}"
              onChange={(event) => update({ templates: parseLines(event.target.value) })}
            />
            <p className="text-sm font-medium">共 {edit.templates.length} 条</p>
            <p className="text-xs text-muted-foreground">
              每行一个不同句式，可直接写无槽位句子；{"{槽位名}"}{" "}
              是可变部分，需在下方槽位定义里声明。模型训练每类至少需要两条不同的展开句子；少于两条会明确失败，不会复制样本伪造评测。
            </p>
          </div>
          <div className="grid gap-1.5 py-2">
            <p className="text-sm font-medium">技能动作</p>
            <select
              className="max-w-xs rounded-md border border-input bg-background px-3 py-2 text-sm"
              value={edit.actionKind}
              onChange={(event) => update({ actionKind: event.target.value })}
            >
              {actionOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
            <p className="text-xs text-muted-foreground">
              能力组合 =
              声明条件与系统能力步骤；已有应答、结束聆听与设备工具动作继续兼容。动作名称与工具名由配置确定，模型不能创建任意
              API 或脚本。
            </p>
          </div>
          {edit.actionKind === "plan" ? (
            <>
              <JsonTextField
                label="条件与步骤（完整 JSON）"
                value={edit.planText}
                error={planError}
                placeholder='{"conditions":[],"steps":[{"api":"replies.say","args":{"text":"你好"}}]}'
                onChange={updatePlanText}
                hint="完整保留条件、步骤与嵌套参数。参数支持 {{slots.名称}}、{{ctx.device_id}}、{{ctx.session_id}}；最多 16 条条件与 16 个步骤。"
              />
              <div className="grid gap-1.5 py-2">
                <FormNotice tone="info">
                  形状检查通过 ≠ 服务端校验通过：能力名 / 条件 / 参数语义以服务端为准（SkillPlan /
                  SkillStep 均 deny_unknown_fields，未知字段会在保存时返回 400
                  纯文本）。分类测试不会运行这些步骤；提醒任务创建、到期投递与设备实际播报是不同环节，需分别验收。
                </FormNotice>
              </div>
            </>
          ) : null}
          {edit.actionKind === "mcp" ? (
            <div className="grid gap-1.5 py-2">
              <p className="text-sm font-medium">设备工具名</p>
              <Input
                className="max-w-md font-mono"
                value={edit.mcpTool}
                placeholder="self.audio_speaker.set_volume"
                onChange={(event) => update({ mcpTool: event.target.value })}
              />
              <p className="text-xs text-muted-foreground">
                设备固件在 MCP 清单里声明的工具名（静态字面量），需精确匹配（清单见「MCP
                工具」页）。写回时以 mcp:&lt;工具名&gt; 编码进
                kind；设备没有声明同名工具则走固定降级话术拒识、不报错崩会话。
              </p>
            </div>
          ) : null}
          {edit.actionKind !== "plan" ? (
            <div className="grid gap-1.5 py-2">
              <p className="text-sm font-medium">应答语</p>
              <Input
                className="max-w-2xl"
                value={edit.reply}
                placeholder="好的，已为你打开灯。"
                onChange={(event) => update({ reply: event.target.value })}
              />
              <p className="text-xs text-muted-foreground">
                命中后要说的话（走 TTS 合成）。留空 = 只执行动作、不出声。
              </p>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

export function NluIntentsEditor(): ReactElement {
  const { getField, setField } = useConfigDraft();
  // draft 中 nlu.intents 的当前引用：外部重建（reload / 回读 / discard）时引用变化。
  const draftIntents = getField("nlu.intents");
  const [items, setItems] = useState<IntentEdit[]>(() => normalizeIntents(draftIntents));
  const [expandedIndex, setExpandedIndex] = useState<number | null>(null);
  // effect 里读最新 items 做比较，避免把 items 加进依赖造成无谓重跑。
  const itemsRef = useRef<IntentEdit[]>(items);
  itemsRef.current = items;

  useEffect(() => {
    // 外部重建检测：draft 里的数组与本地副本构建的 wire 不一致时才重置副本。
    // 自己 commit 造成的变化内容一致（JSON 相等），不打断编辑中的文本中间态。
    const wireFromLocal = itemsRef.current.map(intentToWire);
    if (!jsonEqual(draftIntents, wireFromLocal)) {
      setItems(normalizeIntents(draftIntents));
    }
  }, [draftIntents]);

  const commit = (next: IntentEdit[]): void => {
    setItems(next);
    // 整段写回：数组作为叶子值，绝不写下标路径。
    setField("nlu.intents", next.map(intentToWire));
  };

  const updateAt = (index: number, next: IntentEdit): void => {
    commit(itemsRef.current.map((item, i) => (i === index ? next : item)));
  };

  const removeAt = (index: number): void => {
    commit(itemsRef.current.filter((_, i) => i !== index));
    if (expandedIndex === index) {
      setExpandedIndex(null);
    }
  };

  const addIntent = (): void => {
    const fresh: IntentEdit = {
      name: "",
      domain: "server",
      templates: [],
      actionKind: "reply",
      mcpTool: "",
      reply: "",
      planText: DEFAULT_PLAN_TEXT,
      lastValidPlan: null,
      sourceIntent: {},
      sourceAction: {},
      sourceName: "",
    };
    const next = [...itemsRef.current, fresh];
    setExpandedIndex(next.length - 1);
    commit(next);
  };

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <p className="min-w-0 flex-1 text-sm font-medium">自定义技能（{items.length} 条）</p>
        <Button type="button" variant="outline" size="sm" onClick={addIntent}>
          <Plus aria-hidden="true" className="size-4" />
          添加技能
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        每个技能由名称、控制域、训练例句与配置动作组成；技能名即模型训练标签。保存态技能目录见下方「保存态技能目录」卡，与本编辑器并存（编辑结果保存并构建后才进入目录）。
      </p>
      {items.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          点「添加技能」填写名称、控制域、训练例句与动作，然后保存配置。无槽位技能无需添加槽位；每类训练至少需要两条不同句子。
        </p>
      ) : (
        <div className="space-y-2">
          {items.map((edit, index) => (
            <IntentBlock
              key={`${edit.sourceName}:${index}`}
              edit={edit}
              expanded={expandedIndex === index}
              onToggle={() => setExpandedIndex((current) => (current === index ? null : index))}
              onChange={(next) => updateAt(index, next)}
              onRemove={() => removeAt(index)}
            />
          ))}
        </div>
      )}
    </div>
  );
}
