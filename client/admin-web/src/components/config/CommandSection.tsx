/**
 * `[command]` 结束与指令配置段（W5 / Config C2）。
 *
 * 字段清单与默认值：`plugins/command/mod.rs:19-45` +
 * `plugins/command/defaults.json`（经 `:131-133` 的 `default_keywords()`、
 * `:137-139` 的 `default_reply()` 读入）。
 *
 * ⛔ 只暴露 3 个可编辑字段：`command.match_mode` / `command.max_chars` 是
 * **弃用兼容字段**（`plugins/command/mod.rs:26-31`「不参与识别」「不限制模型输入或
 * 训练词表」；`plugins/registry/fields_command.rs:1`「旧规则字段只反序列化兼容，
 * 不再向管理表单暴露」）。这里既不渲染、也不写进 patch —— 暴露会让用户误以为改了有效。
 *
 * 保存只靠 `setField("点分路径", value)` 由 `diffPatch` 生成最小补丁，
 * 不做旧 Kotlin 的「整段回传」（见 `ConfigFormDraft.kt:84-92` 与
 * `server/src/gateway/ws/config.rs:400-430` 的部分更新语义）。
 */

import type { ReactElement } from "react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { FormNotice } from "@/components/form/FormNotice";
import { StringListField, stringListOf } from "@/components/form/StringListField";
import { SwitchField } from "@/components/form/SwitchField";
import { TextField } from "@/components/form/TextField";
import { useConfigDraft } from "@/context/ConfigDraftContext";

export function CommandSection(): ReactElement {
  const { getField } = useConfigDraft();
  const enabled = getField("command.enabled") === true;
  const keywordCount = stringListOf(getField("command.keywords")).filter(
    (line) => line.trim().length > 0,
  ).length;
  const emptyKeywords = enabled && keywordCount === 0;

  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="command" />

      <FormNotice tone="info">
        关键词是设备域技能 end_session（断开对话）的训练样本，直接进入统一训练目录，不是运行期精确匹配词表（plugins/command/mod.rs:22-25）。修改后须重新训练验证。
      </FormNotice>

      {emptyKeywords ? (
        <FormNotice tone="warning">
          已启用结束聆听技能，但训练样例为空。保存不会失败，但服务端会把该技能标记为 Degraded（/api/plugins 可见降级原因），运行时不执行结束聆听：请至少填一行完整训练句，或关闭总开关。
        </FormNotice>
      ) : null}

      <SectionCard
        title="结束与指令"
        description="end_session 设备域技能的训练语料与告别语；字段来自 [command]（plugins/command/mod.rs:19-45）。"
      >
        <div className="divide-y divide-border">
          <SwitchField
            path="command.enabled"
            label="启用结束聆听技能"
            hint="默认 true（plugins/command/mod.rs:20-21、:39）。关闭仅移除此技能，不影响普通聊天。"
          />

          <StringListField
            path="command.keywords"
            label="结束聆听训练样例"
            hint="每行一个完整训练句，默认「退下 / 闭嘴 / 关闭」（plugins/command/defaults.json、plugins/registry/fields_command.rs:12-14）。空行会被丢弃；词表不允许出现花括号占位符（plugins/command/mod.rs:80-88）。"
            placeholder={"退下\n闭嘴\n关闭"}
          />

          <TextField
            path="command.reply"
            label="告别语"
            hint="默认「好的，我先退下了。」（plugins/command/defaults.json）；留空只执行结束动作。"
            placeholder="好的，我先退下了。"
          />
        </div>
      </SectionCard>
    </div>
  );
}
