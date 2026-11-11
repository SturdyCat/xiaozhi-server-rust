/**
 * `[wakeup]` 唤醒问候语配置段（W5 / Config C2）。
 *
 * 字段清单与默认值：`plugins/wakeup/mod.rs:44-93`（DEFAULT_GREETING「我在」:44、
 * enabled/persist 默认 true :77-83、:85-93）。
 * 换行分句、每次唤醒随机选一句：`:60-64`、`:101-104`；
 * `persist` 依赖 `[tts].cache_entries > 0`：`:65-68`、
 * `plugins/registry/fields_wake.rs:41-44`、`plugins/registry/validate.rs:172-180`。
 * 启用但空池属于可选能力的 `Degraded`（`:106-115`、
 * `plugins/registry/impls/descriptors.rs:460`），不是硬错误 —— 这里只做行内提示，
 * 不阻断保存、不做硬校验。
 *
 * `wakeup` 不在 `APP_SECTION_HOT`（plugins/registry/mod.rs:298-316），
 * `SectionHotHint` 取不到就返回 null，不自造档位文案。
 */

import type { ReactElement } from "react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { FormNotice } from "@/components/form/FormNotice";
import { SwitchField } from "@/components/form/SwitchField";
import { TextAreaField } from "@/components/form/TextAreaField";
import { useConfigDraft } from "@/context/ConfigDraftContext";
import type { JsonValue } from "@/lib/configApi";

/** 按换行拆分问候语池、逐行 trim、丢空行（对齐 wakeup/mod.rs:101-104）。 */
function greetingLines(value: JsonValue | undefined): string[] {
  if (typeof value !== "string") {
    return [];
  }
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

export function WakeupSection(): ReactElement {
  const { getField } = useConfigDraft();
  const enabled = getField("wakeup.enabled") === true;
  const persist = getField("wakeup.persist") === true;
  const emptyPool = enabled && greetingLines(getField("wakeup.greeting")).length === 0;

  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="wakeup" />

      {emptyPool ? (
        <FormNotice tone="warning">
          已启用唤醒问候语，但问候语池为空（全为空行）。这属于可选能力的 Degraded，不是硬错误：唤醒后不会问候，其余语音服务照常（plugins/wakeup/mod.rs:106-115）。请填至少一句（如「我在」），或关闭总开关。
        </FormNotice>
      ) : null}

      <SectionCard
        title="唤醒与问候"
        description="唤醒后立刻说的话与音频持久化；字段来自 [wakeup]（plugins/wakeup/mod.rs:53-93）。"
      >
        <div className="divide-y divide-border">
          <SwitchField
            path="wakeup.enabled"
            label="启用唤醒问候语"
            hint="默认 true（plugins/wakeup/mod.rs:56-59、:88）；关闭时零开销，行为与未装该插件一致。"
          />

          <TextAreaField
            path="wakeup.greeting"
            label="问候语"
            hint="默认「我在」。支持多句：换行分隔，每次唤醒随机选一句（单行 = 单句）；留空或全是空白行 = 不问候（plugins/wakeup/mod.rs:60-64、:101-104）。每句软上限 30 字（GREETING_SOFT_LIMIT :51、校验 :135-144），注册表建议 ≤ 10 字（fields_wake.rs:30-32）：问候语播放期间上行音频会在缓冲区排队，太长会推迟用户第一句话的处理。"
            placeholder="我在"
            rows={4}
          />

          <SwitchField
            path="wakeup.persist"
            label="持久化复用（重启后免合成）"
            hint="默认 true（plugins/wakeup/mod.rs:69-70、:81-83）。把成品音频写到 hot-audio/ 目录，进程重启后直接读盘复用。"
          />
          {persist ? (
            <div className="py-2">
              <FormNotice tone="info">
                持久化复用依赖 [tts].cache_entries &gt; 0：缓存被关掉时音频无法落盘，每次唤醒都会重新合成（plugins/wakeup/mod.rs:65-68、plugins/registry/validate.rs:172-180）。这里只做提示，不做跨段硬校验。
              </FormNotice>
            </div>
          ) : null}
        </div>
      </SectionCard>
    </div>
  );
}
