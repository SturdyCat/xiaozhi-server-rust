/**
 * `[server]` 配置段（W4 / Config C1）。
 *
 * 字段清单：`server/src/config.rs:206-221`；默认值：`:537-539`、`:578-587`；
 * advanced 标记：`plugins/registry/projection.rs:113-114`。
 *
 * ⚠️ 空值语义：`server.expected_token` **不在** `SECRET_PATHS`
 * （`server/src/gateway/ws/config.rs:74-86`），GET 明文返回、`""` = 不校验
 * `Authorization`（`config.rs:211-213`）。所以这里用 `TextField type="password"`
 * 直接把空串写进草稿，**不能**用 `SecretField` 的「留空保留」。
 */

import type { ReactElement } from "react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { FormNotice } from "@/components/form/FormNotice";
import { NumberField } from "@/components/form/NumberField";
import { TextField } from "@/components/form/TextField";
import { useAdvancedMode } from "@/context/AdvancedModeContext";

export function ServerSection(): ReactElement {
  const { advancedMode } = useAdvancedMode();

  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="server" />
      <SectionCard
        title="服务"
        description="监听端口、Bearer 鉴权与管理页面静态目录。字段来自 [server]（server/src/config.rs:206-221）。"
      >
        <div className="divide-y divide-border">
          <NumberField
            path="server.port"
            label="监听端口"
            hint="服务器固定监听 0.0.0.0，唯一可配的是端口（u16，默认 8000）。"
            min={1}
            max={65535}
            placeholder="8000"
          />

          <TextField
            path="server.expected_token"
            label="期望的 Bearer token"
            type="password"
            placeholder="留空表示不校验 Authorization"
            hint="该字段不属于服务端 SECRET_PATHS，GET 会明文返回；为空字符串表示不校验 Authorization。"
          />
          <div className="py-2">
            <FormNotice tone="warning">
              修改后所有客户端需同步更新 token；留空表示不校验 Authorization。
            </FormNotice>
          </div>

          <NumberField
            path="server.worker_threads"
            label="tokio worker 线程数"
            hint="异步运行时线程数，默认 2（避免占满低功耗主机）；u32，至少为 1。"
            advanced
            min={1}
            placeholder="2"
          />

          <TextField
            path="server.admin_dir"
            label="管理页面静态目录"
            hint="server 在 / 直接托管该目录；为空或目录不存在时返回友好提示。默认 ../client/apps/h5App/web。"
            advanced
            placeholder="../client/apps/h5App/web"
          />
          {advancedMode ? (
            <div className="py-2">
              <FormNotice tone="warning">
                修改静态目录后当前管理页面可能无法再加载，需确认新目录内已放置构建产物。
              </FormNotice>
            </div>
          ) : null}
        </div>
      </SectionCard>
    </div>
  );
}
