/**
 * `[echomuse]` EchoMuse 设备接入配置段（W9 / Config C6）。
 *
 * 字段清单与默认值：`server/src/device/echomuse/config.rs:31-72`（7 字段：
 * engine / enabled / token / ble_proxy / plain_port / welcome / music_dir）。
 * 整段属 **RestartOnly**（config.rs:3：任何字段改动重启后才生效）——档位提示由
 * schema `app_sections.hot_hint` 经既有 `SectionHotHint` 下发，**不手写档位文案**。
 *
 * 暴露 6 字段（延续 W8「schema 全量、旧端缺口补上」口径；旧端 ConfigEchoMuseCard.kt
 * 只暴露 3 个）：enabled / token / welcome / engine / ble_proxy / plain_port。
 * **music_dir 本轮不暴露**：schema `FieldKind::Dir`，admin-web 无路径选择控件——
 * 文件尾 FormNotice 说明，列入后续补全清单（禁止拿 TextField 假装路径选择器）。
 *
 * fail-closed 是**字段级**的：该段是 `#[cfg(feature)]` 门控的 Config 字段，pure-base
 * 构建下 GET /api/config 回包里**根本没有 `echomuse` key**——此时显示 UnavailableNotice
 * （「该能力在当前构建未编入」），**绝不显示空表单假装可配置**
 * （判据：draft 中 `echomuse === undefined`）。
 *
 * 禁止项：不调任何 `/em/*`、`/api/echomuse/*` 端点（设备运行面，全部经 auth_ok，
 * 留 W12）；不做「管理设备」跳转（依赖 devicehub，留 W12）；不出现「registered」字样
 * （那是 WASM 插件生命周期语义，与本段无关）。
 */

import type { ReactElement } from "react";
import { SectionCard } from "@/components/config/SectionCard";
import { SectionHotHint } from "@/components/config/SectionHotHint";
import { UnavailableNotice } from "@/components/feedback/UnavailableNotice";
import { FormNotice } from "@/components/form/FormNotice";
import { NumberField } from "@/components/form/NumberField";
import { SecretField } from "@/components/form/SecretField";
import { SelectField } from "@/components/form/SelectField";
import { SwitchField } from "@/components/form/SwitchField";
import { TextAreaField } from "@/components/form/TextAreaField";
import type { SelectOption } from "@/components/form/types";
import { useConfigDraft } from "@/context/ConfigDraftContext";

/** `[echomuse].engine` 实现 id；`wasm` 别名由服务端 normalize 收敛为 wasm-policy，
 * 未知值由 SelectField 只告警、不纠正（config.rs:17-26）。 */
const ENGINE_OPTIONS: SelectOption[] = [
  { value: "native", label: "宿主原生控制面（native）" },
  { value: "wasm-policy", label: "WASM 控制策略 + 原生数据面（wasm-policy）" },
];

export function EchomuseSection(): ReactElement {
  const { getField } = useConfigDraft();
  // 字段级 fail-closed：pure-base 构建的 GET /api/config 回包里没有 echomuse 键。
  const available = getField("echomuse") !== undefined;
  const enabled = getField("echomuse.enabled") === true;

  return (
    <div className="space-y-4">
      <SectionHotHint sectionId="echomuse" />

      {!available ? (
        <UnavailableNotice
          title="EchoMuse 能力未编入当前构建"
          description="GET /api/config 回包里没有 [echomuse] 配置段（通常是构建未启用 native-plugins）。该状态下没有可配置的字段；请改用启用 native-plugins 的服务端构建。"
        />
      ) : (
        <SectionCard
          title="EchoMuse · 设备接入（[echomuse]）"
          description="EchoMuse 设备入口开关与凭据；字段来自 server/src/device/echomuse/config.rs:31-72。整段重启后才生效。"
        >
          <div className="divide-y divide-border">
            <SwitchField
              path="echomuse.enabled"
              label="启用 EchoMuse 设备入口"
              hint="启用后挂载 em 路由并与同网段设备建立连接；关闭时 EchoMuse 页只显示「未启用」提示，不挂 em 路由、不广播 mDNS。"
            />
            <SecretField
              path="echomuse.token"
              hasPath="echomuse.has_token"
              label="接入令牌"
              hint="同网段设备接入凭据（X-Em-Token）。留空 = 不校验；留空保存不会覆盖已设置的值。"
            />
            <TextAreaField
              path="echomuse.welcome"
              label="连接欢迎语"
              rows={3}
              hint="设备接入后立刻说一次（默认含使用提示）。留空 = 不说；改后需重启生效。每次会话建立（含断线重连）都会说一遍，重连频繁时请自行取舍。"
            />
            <SelectField
              path="echomuse.engine"
              label="实现 id"
              options={ENGINE_OPTIONS}
              hint="native = 宿主原生控制面；wasm-policy = WASM 控制策略（历史值 wasm 由服务端自动收敛）。服务端返回未知值时界面只告警、不自动纠正。"
            />
            <SwitchField
              path="echomuse.ble_proxy"
              label="蓝牙代理"
              hint="默认关闭：关闭时 ack 不声明 ble_connect / ble_adverts_data，0x06 / 0x08 帧按未知忽略、管理面 BLE 端点 404。开启后仅对 register 声明 ble_connect 的固件实际生效。"
            />
            <NumberField
              path="echomuse.plain_port"
              label="控制器专用端口"
              min={0}
              max={65535}
              placeholder="8767"
              hint="u16，默认 8767（官方明文 controller 端口）：>0 时额外在此端口监听设备面路由（/em/* + /shell），mDNS 也广播它；0 = 关闭（仅主端口）。绑定失败只告警并回退广播主端口。"
            />

            {!enabled ? (
              <p className="py-2 text-xs text-muted-foreground">
                未启用：EchoMuse 页只显示「未启用」提示，不挂 em 路由、不广播 mDNS。
              </p>
            ) : null}
          </div>
        </SectionCard>
      )}

      {available ? (
        <FormNotice tone="info">
          「music_dir」（音乐文件根目录，默认 /data/music）本轮未暴露：需要目录选择控件（schema FieldKind::Dir），已列入后续补全清单；当前可在配置文件中直接编辑该键。
        </FormNotice>
      ) : null}
    </div>
  );
}
