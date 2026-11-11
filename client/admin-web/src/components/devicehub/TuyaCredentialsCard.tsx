/**
 * 涂鸦云凭证卡（W12 devicehub 齿轮弹层内嵌；TuyaPage.kt:36-118 逐字移植）。
 *
 * **凭证唯一录入路径是 `POST /api/tuya/credentials`**（C6 铁律）：本组件不接
 * ConfigDraftContext / setField / SaveBar / PUT /api/config 主链；secret 任何情况下
 * 不回显——保存成功后立即清空草稿（TuyaState.kt:17-21）。清除走行内二次确认。
 */

import { useState, type ReactElement } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useToast } from "@/components/feedback/ToastHost";
import { clearTuyaCredentials, saveTuyaCredentials, type TuyaStatus } from "@/lib/tuyaApi";

/** 区域四选项（TuyaPage.kt:63-72 逐字；REGIONS cn/us/eu/in）。 */
const REGIONS: { value: string; label: string }[] = [
  { value: "cn", label: "中国（cn）" },
  { value: "us", label: "美洲（us）" },
  { value: "eu", label: "欧洲（eu）" },
  { value: "in", label: "印度（in）" },
];

export interface TuyaCredentialsCardProps {
  /** 当前涂鸦状态（configured/access_id_masked/region 展示用）；null = 状态未加载。 */
  status: TuyaStatus | null;
  /** tuya.enabled=false 时整卡禁用（/api/tuya/* 全部 404，凭证录入同样不可用）。 */
  disabled: boolean;
  /** 凭证变化后由父级刷新 status 与设备目录。 */
  onChanged: () => void;
}

export function TuyaCredentialsCard({
  status,
  disabled,
  onChanged,
}: TuyaCredentialsCardProps): ReactElement {
  const toast = useToast();
  const configured = status?.configured === true;
  const [region, setRegion] = useState<string>(status?.region || "cn");
  const [accessId, setAccessId] = useState("");
  const [accessSecret, setAccessSecret] = useState("");
  const [saving, setSaving] = useState(false);
  const [clearArmed, setClearArmed] = useState(false);

  const handleSave = async (): Promise<void> => {
    // TuyaState.kt:200-229 逐字校验：空 id 拦截；未配置时空 secret 拦截。
    if (accessId.trim() === "") {
      toast.show("请先填写 access_id", "error");
      return;
    }
    if (!configured && accessSecret === "") {
      toast.show("请填写 access_secret（已配置过可留空保留旧值）", "error");
      return;
    }
    setSaving(true);
    try {
      const message = await saveTuyaCredentials({
        region,
        accessId: accessId.trim(),
        accessSecret,
      });
      // 成功后立即清空 secret 草稿（secret 永不回显铁律）。
      setAccessSecret("");
      toast.show(message || "凭证已保存（secret 不再回显）", "success");
      onChanged();
    } catch (error) {
      toast.show(error instanceof Error ? error.message : String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  const handleClear = async (): Promise<void> => {
    setSaving(true);
    try {
      const message = await clearTuyaCredentials();
      toast.show(message || "凭证已清除", "success");
      setClearArmed(false);
      onChanged();
    } catch (error) {
      toast.show(error instanceof Error ? error.message : String(error), "error");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-3 rounded-lg border p-3">
      <div className="flex items-center justify-between gap-2">
        <p className="text-sm font-medium">涂鸦云凭证</p>
        <p className="text-xs text-muted-foreground">
          {configured
            ? `已配置 · ${status?.accessIdMasked ?? ""} · ${status?.region ?? ""}`
            : "未配置"}
        </p>
      </div>
      <div className="space-y-1.5">
        <Label className="text-xs">区域</Label>
        <Select
          value={region}
          onValueChange={(value) => setRegion(value)}
          disabled={disabled || saving}
        >
          <SelectTrigger className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {REGIONS.map((item) => (
              <SelectItem key={item.value} value={item.value}>
                {item.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="space-y-1.5">
        <Label className="text-xs" htmlFor="tuya-access-id">
          access_id
        </Label>
        <Input
          id="tuya-access-id"
          value={accessId}
          placeholder="涂鸦 IoT 平台云开发项目的 access_id"
          disabled={disabled || saving}
          onChange={(event) => setAccessId(event.target.value)}
        />
      </div>
      <div className="space-y-1.5">
        <Label className="text-xs" htmlFor="tuya-access-secret">
          {configured ? "access_secret（留空保留旧值）" : "access_secret"}
        </Label>
        <Input
          id="tuya-access-secret"
          type="password"
          value={accessSecret}
          placeholder={
            configured ? "留空保存 = 保留旧值" : "涂鸦 IoT 平台云开发项目的 access_secret"
          }
          disabled={disabled || saving}
          onChange={(event) => setAccessSecret(event.target.value)}
        />
        {configured ? (
          <p className="text-xs text-muted-foreground">
            access_secret 已配置（不再回显）。留空保存 = 保留旧值；更换请输入新值。
          </p>
        ) : null}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          size="sm"
          disabled={disabled || saving}
          onClick={() => void handleSave()}
        >
          {saving ? "保存中…" : "保存凭证"}
        </Button>
        {configured ? (
          clearArmed ? (
            <>
              {/* 行内二次确认（TuyaPage.kt:101-115 逐字） */}
              <span className="text-xs text-destructive">确认清除？</span>
              <Button
                type="button"
                variant="destructive"
                size="sm"
                disabled={saving}
                onClick={() => void handleClear()}
              >
                清除凭证
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={saving}
                onClick={() => setClearArmed(false)}
              >
                取消
              </Button>
            </>
          ) : (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={disabled || saving}
              onClick={() => setClearArmed(true)}
            >
              清除凭证
            </Button>
          )
        ) : null}
      </div>
    </div>
  );
}
