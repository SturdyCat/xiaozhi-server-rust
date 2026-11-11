/**
 * 配置草稿与基线的最小差异计算。
 *
 * 服务端 `PUT /api/config` 是**增量合并**（出现即覆盖，未出现保持现值），
 * 因此这里只产出变化的叶子字段：
 * - 对象递归比较；
 * - 数组、标量与类型变化整体替换；
 * - draft 新增的键会出现在 patch 里。
 *
 * ⚠️ 已知限制：服务端没有“删除键”语义（`null` 表示清除并回落默认值，不是删键），
 * 所以 draft 里被删掉的键**不会**进入 patch，保存后仍保留 baseline 的旧值。
 * 需要清空某个字段时必须显式写入 `null`，不能依赖删键。
 */

import type { JsonObject, JsonValue } from "@/lib/configApi";

/** 判断是否为普通 JSON 对象（数组不算）。 */
export function isPlainObject(value: JsonValue | undefined): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 深度比较两个 JSON 值；对象键序不影响结果。 */
function deepEqual(a: JsonValue | undefined, b: JsonValue | undefined): boolean {
  if (a === b) {
    return true;
  }
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) {
      return false;
    }
    return a.every((item, index) => deepEqual(item, b[index]));
  }
  if (isPlainObject(a) && isPlainObject(b)) {
    const aKeys = Object.keys(a);
    const bKeys = Object.keys(b);
    if (aKeys.length !== bKeys.length) {
      return false;
    }
    return aKeys.every(
      (key) => Object.prototype.hasOwnProperty.call(b, key) && deepEqual(a[key], b[key]),
    );
  }
  return false;
}

/**
 * 计算 `baseline -> draft` 的最小 patch（只含变化叶子）。
 *
 * 只遍历 draft 的键：draft 删除的键不会被表达（见文件头的限制说明）。
 */
export function diffPatch(baseline: JsonObject, draft: JsonObject): JsonObject {
  const patch: JsonObject = {};
  for (const key of Object.keys(draft)) {
    const baseValue: JsonValue | undefined = baseline[key];
    const draftValue: JsonValue = draft[key];
    if (isPlainObject(baseValue) && isPlainObject(draftValue)) {
      const childPatch = diffPatch(baseValue, draftValue);
      if (Object.keys(childPatch).length > 0) {
        patch[key] = childPatch;
      }
    } else if (!deepEqual(baseValue, draftValue)) {
      // 数组 / 标量 / 类型变化：整体替换。
      patch[key] = draftValue;
    }
  }
  return patch;
}

/** 统计 patch 里的叶子数（即“多少处修改”）。 */
function countLeaves(patch: JsonObject): number {
  let total = 0;
  for (const value of Object.values(patch)) {
    total += isPlainObject(value) ? countLeaves(value) : 1;
  }
  return total;
}

/** 返回 draft 相对 baseline 的变化叶子数量（0 表示无改动）。 */
export function countDirty(draft: JsonObject, baseline: JsonObject): number {
  return countLeaves(diffPatch(baseline, draft));
}

/**
 * 把 patch 展开成叶子点分路径清单（`{ aiui: { enabled: true } }` → `["aiui.enabled"]`）。
 *
 * 供简洁模式的「含高级字段」二次确认使用：路径格式与服务端
 * `GET /api/config/schema` 的 `advanced_fields` 完全一致，可直接比对。
 */
export function dirtyPathsOf(patch: JsonObject): string[] {
  const paths: string[] = [];
  const walk = (node: JsonObject, prefix: string): void => {
    for (const [key, value] of Object.entries(node)) {
      const path = prefix === "" ? key : `${prefix}.${key}`;
      if (isPlainObject(value)) {
        walk(value, path);
      } else {
        paths.push(path);
      }
    }
  };
  walk(patch, "");
  return paths;
}
