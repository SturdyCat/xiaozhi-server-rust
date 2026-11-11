/**
 * EchoMuse 设备详情弹窗（W12 devicehub；R3/R4 拍板形态：`/em/` iframe 嵌入单设备，
 * CDN 直连不 vendor）。对齐旧端 EchoMuseOfficialViewHost.kt 的容器契约：
 * - sandbox `allow-scripts allow-same-origin allow-forms allow-downloads`、无 window.open；
 * - `referrerpolicy="no-referrer"`（URL 含管理台令牌，与 em/index.html `no-referrer`
 *   meta 双保险）；
 * - 父容器给 iframe 明确高度（em/README.md:147）；
 * - 未审批设备忽略 tab（host.js 继续显示官方审批表单）——本弹窗一律不传 tab，
 *   未批准设备由官方页审批表单承接；
 * - R4 已知限制：第三方库/字体走公共 CDN，内网/离线白屏——iframe 外方常驻诚实文案
 *   （EM_EMBED_CDN_NOTICE），不伪装成功能故障（外层读不到 iframe 内 CDN 失败原因）；
 * - 点遮罩不关闭（Dialog onInteractOutside preventDefault）。
 */

import { type ReactElement } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Badge } from "@/components/ui/badge";
import {
  buildEmEmbedUrlWithAdminToken,
  EM_EMBED_CDN_NOTICE,
  EM_IFRAME_SANDBOX,
} from "@/lib/emEmbed";
import type { EchomuseDevice } from "@/lib/echomuseApi";

export interface EchoMuseDetailDialogProps {
  device: EchomuseDevice | null;
  onClose: () => void;
}

export function EchoMuseDetailDialog({
  device,
  onClose,
}: EchoMuseDetailDialogProps): ReactElement {
  const open = device !== null;
  // device 变化时 key 随之变化：销毁旧浏览上下文再挂新 iframe（旧端 key(url) 口径）。
  const embedUrl =
    device !== null
      ? buildEmEmbedUrlWithAdminToken({ device: device.deviceId })
      : "";

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          onClose();
        }
      }}
    >
      <DialogContent
        className="h-[85vh] max-w-4xl overflow-hidden"
        onInteractOutside={(event) => {
          // 点遮罩不关闭（对齐旧端 overlay dismissOnOutside=false）。
          event.preventDefault();
        }}
      >
        <DialogHeader>
          <DialogTitle className="flex flex-wrap items-center gap-2">
            {device?.label || device?.deviceId || ""}
            {device ? (
              <Badge variant="outline">{device.approved ? "已批准" : "待批准"}</Badge>
            ) : null}
          </DialogTitle>
        </DialogHeader>
        {device !== null ? (
          <div className="flex h-full min-h-0 flex-col gap-2">
            <p className="shrink-0 text-xs text-muted-foreground">{EM_EMBED_CDN_NOTICE}</p>
            <iframe
              key={device.deviceId}
              src={embedUrl}
              title={`EchoMuse 官方管理页（${device.label || device.deviceId}）`}
              className="min-h-0 w-full flex-1 rounded-md border bg-background"
              sandbox={EM_IFRAME_SANDBOX}
              referrerPolicy="no-referrer"
            />
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
