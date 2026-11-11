/**
 * A2UI v0.1 动态 UI 文档解析器（纯函数，零 React 依赖）。
 *
 * 逐条移植自 `client/shared/.../contracts/PluginUi.kt`（316 行全读核实），规则一致：
 *
 * ## 数据源与契约
 *
 * 字段来自 `GET /api/plugins` 的 `wasm.plugins[].registration.ui`：宿主是**透传管道**
 * （装载期只校验「单个 JSON 对象」与 ≤256KiB，不解释内容），仅 **Active** 插件投影，
 * 其余 phase / 未声明投影 `null`。文档形状：`{a2ui:"0.1", pages:[{id,title,kind,tree,bindings}]}`；
 * `tree` 是嵌套节点树（0.1 语法是旧扁平两页形态的超集，form/panel 原样保留兼容）。
 *
 * ## 组件（全部业务无关）
 *
 * - 容器（只做布局）：`card`（带标题、单 child）、`column`（children 数组）；
 * - 叶子：`text` / `hint`（各 ≤500 字符，超出**丢弃**不截断）；`field`（单字段引用）、
 *   `form`（旧扁平字段组 = column of field）；`actions`（动作 id 数组）、`panel`
 *   （旧扁平动作组 = actions）；`input`（页内临时输入，不落配置）、`result`
 *   （最近一次动作结果占位）。
 *
 * ## fail-closed 规则（声明清单是唯一权威）
 *
 * 1. `a2ui` 版本必须**显式等于** "0.1"：缺版本 / 更高版本 → **整份文档作废**（不猜形状）；
 * 2. `field`/`form` 引用必须命中 `registration.config_fields[].key`；`actions`/`panel`
 *    的动作 id 必须命中 `registration.actions[].id`——未命中**逐条丢弃**；label/kind/
 *    help/danger 元数据一律取注册声明，文档内同名键只是展示副本，不能改写文案或
 *    把危险动作洗白；
 * 3. `input.id` 必须匹配 `[a-z0-9_-]{1,32}` 且**页内唯一**（同 id 第二个丢弃），label
 *    必填非空——违规丢弃该节点；
 * 4. 未知 `type` 丢弃该节点与子树（单个未知组件不作废整份文档）；card 无有效 child /
 *    column 无有效 children 丢弃该容器；
 * 5. 容器嵌套深度 ≤4（第 5 层容器整体丢弃）；页数 ≤8、每页叶子 ≤64（超界按文档顺序
 *    截断）；页内无任何有效叶子丢页；全部页被丢弃 → null（声明式回退页）；
 * 6. `bindings`（`{"config":[key],"actions":[id]}`）仅被容忍不解释——渲染以 tree 为
 *    布局源，解析期只认 tree 内命中声明的引用。
 */

import type {
  PluginAction,
  PluginConfigField,
  PluginRegistration,
  WasmSnapshot,
} from "@/lib/pluginsApi";

/** 当前支持的 A2UI 版本；显式等于才解析，否则整份作废。 */
export const A2UI_SUPPORTED_VERSION = "0.1";

/** 页数上限（超界按文档顺序截断）。 */
const MAX_PAGES = 8;

/** 每页叶子上限（超界按文档顺序截断）。 */
const MAX_TREE_LEAVES = 64;

/** 容器（card/column）嵌套深度上限：第 5 层容器整体丢弃。 */
const MAX_CONTAINER_DEPTH = 4;

/** text/hint 字符上限：超出丢弃该节点（不截断——插件文档不该被静默改写）。 */
const MAX_TEXT_CHARS = 500;

/** `input.id` 形状：小写字母/数字/下划线/连字符，1..32 字符。 */
const INPUT_ID_PATTERN = /^[a-z0-9_-]{1,32}$/;

/** 判断是否为普通 JSON 对象（数组与 null 不算）。 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 读字符串键（必须是字符串原始值；数字/布尔不算）。 */
function readString(raw: Record<string, unknown>, key: string): string | null {
  const value = raw[key];
  return typeof value === "string" ? value : null;
}

/** 解析后的组件树节点（fail-closed 过滤后的引用集；元数据已解析为声明对象）。 */
export type PluginUiNode =
  | { readonly type: "card"; readonly title: string; readonly child: PluginUiNode }
  | { readonly type: "column"; readonly children: PluginUiNode[] }
  | { readonly type: "text"; readonly text: string }
  | { readonly type: "hint"; readonly text: string }
  | { readonly type: "field"; readonly field: PluginConfigField }
  | { readonly type: "form"; readonly fields: PluginConfigField[] }
  | { readonly type: "actions"; readonly actions: PluginAction[] }
  | { readonly type: "panel"; readonly actions: PluginAction[] }
  | {
      readonly type: "input";
      readonly id: string;
      readonly label: string;
      readonly placeholder: string;
    }
  | { readonly type: "result" };

/** 文档的一页；`kind` 是插件自述（如 `config` / `test`），渲染器只做展示标注。 */
export interface PluginUiPage {
  readonly id: string;
  readonly title: string;
  readonly kind: string;
  readonly tree: PluginUiNode;
  /** 解析期收集的页内合法输入 id（动作 payload 组装用：页面含 input 时 payload = 全部输入值对象）。 */
  readonly inputIds: readonly string[];
}

/** 解析成功的动态 UI 文档；null = 走声明式回退页。 */
export interface PluginUiDocument {
  readonly version: string;
  readonly pages: readonly PluginUiPage[];
}

/** 单页解析上下文：叶子预算 + input id 页内唯一集。 */
interface PageCtx {
  leaves: number;
  readonly inputIds: Set<string>;
}

function takeLeaf(ctx: PageCtx): boolean {
  if (ctx.leaves >= MAX_TREE_LEAVES) {
    return false;
  }
  ctx.leaves += 1;
  return true;
}

/**
 * 递归解析一个节点。[depth] 是该节点上方的容器链长度（根调用为 0）：容器入链前校验
 * `depth + 1 ≤ MAX_CONTAINER_DEPTH`，叶子不占深度；节点解析失败返回 null 由上层丢弃
 * （card 失去 child 即整体丢弃、column 过滤无效孩子）。
 */
function parseNode(
  obj: Record<string, unknown>,
  depth: number,
  ctx: PageCtx,
  fields: readonly PluginConfigField[],
  actions: readonly PluginAction[],
): PluginUiNode | null {
  const type = readString(obj, "type");
  switch (type) {
    case "card": {
      if (depth + 1 > MAX_CONTAINER_DEPTH) {
        return null;
      }
      const title = readString(obj, "title");
      if (title === null || title.trim() === "") {
        return null;
      }
      const childRaw = obj["child"];
      if (!isRecord(childRaw)) {
        return null;
      }
      const child = parseNode(childRaw, depth + 1, ctx, fields, actions);
      return child === null ? null : { type: "card", title, child };
    }
    case "column": {
      if (depth + 1 > MAX_CONTAINER_DEPTH) {
        return null;
      }
      const rows = obj["children"];
      if (!Array.isArray(rows)) {
        return null;
      }
      const children: PluginUiNode[] = [];
      for (const row of rows) {
        if (!isRecord(row)) {
          continue;
        }
        const child = parseNode(row, depth + 1, ctx, fields, actions);
        if (child !== null) {
          children.push(child);
        }
      }
      return children.length === 0 ? null : { type: "column", children };
    }
    case "text": {
      const text = readString(obj, "text");
      if (text === null || text.length > MAX_TEXT_CHARS || !takeLeaf(ctx)) {
        return null;
      }
      return { type: "text", text };
    }
    case "hint": {
      const text = readString(obj, "text");
      if (text === null || text.length > MAX_TEXT_CHARS || !takeLeaf(ctx)) {
        return null;
      }
      return { type: "hint", text };
    }
    case "field": {
      const key = readString(obj, "key");
      if (key === null || key.trim() === "" || !takeLeaf(ctx)) {
        return null;
      }
      // fail-closed：只渲染注册声明过的配置字段；元数据取声明，文档不能改写。
      const declared = fields.find((item) => item.key === key);
      return declared ? { type: "field", field: declared } : null;
    }
    case "form": {
      const rows = obj["fields"];
      if (!Array.isArray(rows)) {
        return null;
      }
      const declared: PluginConfigField[] = [];
      for (const row of rows) {
        if (!isRecord(row)) {
          continue;
        }
        const key = readString(row, "key");
        if (key === null || key.trim() === "" || !takeLeaf(ctx)) {
          continue;
        }
        const field = fields.find((item) => item.key === key);
        if (field) {
          declared.push(field);
        }
      }
      return declared.length === 0 ? null : { type: "form", fields: declared };
    }
    case "actions": {
      const rows = obj["actions"];
      if (!Array.isArray(rows)) {
        return null;
      }
      const declared: PluginAction[] = [];
      for (const row of rows) {
        if (typeof row !== "string" || row.trim() === "" || !takeLeaf(ctx)) {
          continue;
        }
        // fail-closed：引用必须命中注册声明；label/danger/help 一律取声明。
        const action = actions.find((item) => item.id === row);
        if (action) {
          declared.push(action);
        }
      }
      return declared.length === 0 ? null : { type: "actions", actions: declared };
    }
    case "panel": {
      const rows = obj["actions"];
      if (!Array.isArray(rows)) {
        return null;
      }
      const declared: PluginAction[] = [];
      for (const row of rows) {
        if (!isRecord(row)) {
          continue;
        }
        const actionId = readString(row, "action");
        if (actionId === null || actionId.trim() === "" || !takeLeaf(ctx)) {
          continue;
        }
        const action = actions.find((item) => item.id === actionId);
        if (action) {
          declared.push(action);
        }
      }
      return declared.length === 0 ? null : { type: "panel", actions: declared };
    }
    case "input": {
      const inputId = readString(obj, "id");
      if (inputId === null || !INPUT_ID_PATTERN.test(inputId)) {
        return null;
      }
      const label = readString(obj, "label");
      if (label === null || label.trim() === "" || !takeLeaf(ctx)) {
        return null;
      }
      // 页内唯一：同 id 的第二个 input 丢弃（首个保留）。
      if (ctx.inputIds.has(inputId)) {
        return null;
      }
      ctx.inputIds.add(inputId);
      return {
        type: "input",
        id: inputId,
        label,
        placeholder: readString(obj, "placeholder") ?? "",
      };
    }
    case "result": {
      if (!takeLeaf(ctx)) {
        return null;
      }
      return { type: "result" };
    }
    // 未知类型：丢弃该节点与子树，不让单个未知组件作废整份文档。
    default:
      return null;
  }
}

function parsePage(
  obj: Record<string, unknown>,
  fields: readonly PluginConfigField[],
  actions: readonly PluginAction[],
): PluginUiPage | null {
  const id = readString(obj, "id");
  if (id === null || id.trim() === "") {
    return null;
  }
  const treeRaw = obj["tree"];
  if (!isRecord(treeRaw)) {
    return null;
  }
  const ctx: PageCtx = { leaves: 0, inputIds: new Set<string>() };
  const tree = parseNode(treeRaw, 0, ctx, fields, actions);
  if (tree === null) {
    return null;
  }
  return {
    id,
    title: readString(obj, "title")?.trim() || id,
    kind: readString(obj, "kind") ?? "",
    tree,
    inputIds: Array.from(ctx.inputIds),
  };
}

/**
 * 解析一份 UI 文档对象（`registration.ui` 投影）。[fields] / [actions] 是同一插件的
 * 注册声明，作为 fail-closed 白名单。任何失败（null / 零有效页）都让调用方走声明式
 * 回退页。
 */
export function parsePluginUiDocument(
  doc: unknown,
  fields: readonly PluginConfigField[],
  actions: readonly PluginAction[],
): PluginUiDocument | null {
  if (!isRecord(doc)) {
    return null;
  }
  // 规则①：版本必须显式等于 "0.1"，缺版本 / 更高版本 → 整份作废。
  if (readString(doc, "a2ui") !== A2UI_SUPPORTED_VERSION) {
    return null;
  }
  const pagesRaw = doc["pages"];
  if (!Array.isArray(pagesRaw)) {
    return null;
  }
  const pages: PluginUiPage[] = [];
  const seenIds = new Set<string>();
  for (const pageRaw of pagesRaw) {
    if (pages.length >= MAX_PAGES) {
      break;
    }
    if (!isRecord(pageRaw)) {
      continue;
    }
    const page = parsePage(pageRaw, fields, actions);
    if (page === null || seenIds.has(page.id)) {
      continue;
    }
    seenIds.add(page.id);
    pages.push(page);
  }
  return pages.length === 0 ? null : { version: A2UI_SUPPORTED_VERSION, pages };
}

/**
 * `wasm` 键内全部插件 UI 文档的一次性解析（绑定同一份快照做 fail-closed 白名单）：
 * 复用同一次拉取、不发第二个请求，也不产生第二份清单语义。仅 Active 插件有
 * `registration.ui` 投影；解析失败/无文档的插件不出现在结果里（走声明式回退页）。
 */
export function parsePluginUiDocs(snapshot: WasmSnapshot): Record<string, PluginUiDocument> {
  const out: Record<string, PluginUiDocument> = {};
  for (const plugin of snapshot.plugins) {
    const registration: PluginRegistration | null = plugin.registration;
    if (registration === null || registration.ui === null) {
      continue;
    }
    const doc = parsePluginUiDocument(
      registration.ui,
      registration.configFields,
      registration.actions,
    );
    if (doc !== null) {
      out[plugin.id] = doc;
    }
  }
  return out;
}
