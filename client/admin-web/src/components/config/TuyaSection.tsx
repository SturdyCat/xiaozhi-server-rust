/**
 * `[tuya]` Tuya 接入配置段（W9 / Config C6）。
 *
 * 字段清单与默认值：`server/src/device/tuya/config.rs:33-56`（6 字段：
 * engine / enabled / region / access_id / access_secret / sync_interval_secs）。
 * **全部热生效**（config.rs:1-6：保存即生效，无需重启）——档位提示由 schema
 * `app_sections.hot_hint` 经既有 `SectionHotHint` 下发，**不手写档位文案**。
 *
 * **只暴露 2 字段**（enabled / sync_interval_secs），与旧端 ConfigTuyaCard.kt 一致：
 * region / access_id / access_secret **一律不渲染**——云凭证唯一录入路径是 devicehub 的
 * `POST /api/tuya/credentials`（不经 PUT 主链），旧端刻意「两套 UI 不互相覆盖」
 * （config.rs:3-4；ConfigTuyaCard.kt:46 逐字提示）。access_secret 虽在 SECRET_PATHS，
 * 本段无凭证渲染，因此无 SecretField。
 *
 * fail-closed 是**字段级**的：该段是 `#[cfg(feature)]` 门控的 Config 字段，pure-base
 * 构建下 GET /api/config 回包里**根本没有 `tuya` key**——此时显示 UnavailableNotice，
 * **绝不显示空表单假装可配置**（判据：draft 中 `tuya === undefined`）。
 *
 * normalize 是静默行为不是 400（config.rs:89-115）：region 非法回落 "cn"、
 * sync_interval 0<秒<60 收敛到 60——文案必须说「服务端会自动收敛」，禁止「保存被拒绝」。
 *
 * 禁止项：不调 `/api/tuya/*`（管理面端点，留 W12）；不做「管理设备」跳转；不出现
 * 「registered」字样。
 */

import type { ReactElement } from "react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { UnavailableNotice } from "@/components/feedback/UnavailableNotice";
import { NumberField } from "@/components/form/NumberField";
import { SwitchField } from "@/components/form/SwitchField";
import { useConfigDraft } from "@/context/ConfigDraftContext";

export function TuyaSection(): ReactElement {
  const { getField } = useConfigDraft();
  // 字段级 fail-closed：pure-base 构建的 GET /api/config 回包里没有 tuya 键。
  const available = getField("tuya") !== undefined;
  const enabled = getField("tuya.enabled") === true;

  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="tuya" />

      {!available ? (
        <UnavailableNotice
          title="Tuya 能力未编入当前构建"
          description="GET /api/config 回包里没有 [tuya] 配置段（通常是构建未启用 native-plugins）。该状态下没有可配置的字段；请改用启用 native-plugins 的服务端构建。"
        />
      ) : (
        <SectionCard
          title="Tuya · 设备接入（[tuya]）"
          description="涂鸦云集成的部署级开关；字段来自 server/src/device/tuya/config.rs:33-56。全部字段保存即生效，无需重启。"
        >
          <div className="divide-y divide-border">
            <SwitchField
              path="tuya.enabled"
              label="启用涂鸦设备"
              hint="启用后开放涂鸦云目录同步与 /api/tuya/* 管理面；关闭即停用全部同步与管理面（保存即生效，无需重启）。局域网控制不依赖云凭证。"
            />
            <NumberField
              path="tuya.sync_interval_secs"
              label="自动同步间隔（秒）"
              min={0}
              placeholder="86400"
              hint="定时拉取涂鸦云设备目录的周期，默认 86400（每天一次）；0 = 仅手动同步，非零最小 60 秒（服务端会自动收敛，不会拒绝保存）。"
            />

            <p className="py-2 text-xs text-muted-foreground">
              云凭证（区域 / access_id / access_secret）在「设备管理」页右上角齿轮里录入，这里不重复编辑。
            </p>

            {!enabled ? (
              <p className="py-2 text-xs text-muted-foreground">
                未启用：/api/tuya/* 全部 404，设备管理页的涂鸦分区只显示未启用提示。
              </p>
            ) : null}
          </div>
        </SectionCard>
      )}
    </div>
  );
}
