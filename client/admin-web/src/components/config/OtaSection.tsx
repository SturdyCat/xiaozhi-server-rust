/**
 * 「固件与 OTA」段（W10 / Config C7）—— 18 段里唯一的**0 配置字段纯动作面板**。
 *
 * ⚠️ ota 不是配置段：`Config` struct 无 ota 字段、projection 三表零 ota、无密钥、
 * `APP_SECTION_HOT` 无 ota。本组件**不接 ConfigDraftProvider / diffPatch / SaveBar /
 * dirtyCount / hot / revision**；0 处 setField / 0 处接 draft；上传不是 config PUT，
 * 绝不出现「配置已保存」类 toast（对齐旧端 `ConfigOtaEntryCard.kt:41`「固件包托管走
 * 独立接口，不随 POST /api/config 回传」）。
 *
 * 结构复刻旧端三卡（`OtaCard.kt`）+ 增强：
 * - 前言卡（挂载点）：解耦声明逐字保留（ConfigOtaEntryCard.kt:40-47 两句）；
 * - 卡 1 规则说明：匹配规则逐字（OtaCard.kt:126-128）+ 摘要（distinct 去重）+ 刷新
 *   （防重入）+ 存储目录；加载失败就地 error 行 + 可行动指引，**不弹一次性 toast**；
 * - 卡 2 上传：文件选择 + **真实百分比进度**（XHR upload.onprogress——旧端不确定态是
 *   fetch 无 onprogress 的妥协，研究已证伪为非服务端能力）+ 手动覆盖 4 字段默认收起
 *   （**URL query 覆盖参数，不是配置字段**，本地 state 不进 draft）+ 失败常驻 Alert
 *   （401 专项指引逐字，toast 3 秒读不完排障指引）+ 成功 toast + metadata_from
 *   **留余地措辞**（`http.rs:135` 只按包内解析成功判定，手填覆盖时仍可能报 package）；
 * - 卡 3 清单表：固件列含 matchNote（三种）+ 指纹行；大小/编译时间右对齐；空清单
 *   可行动解释逐字（OtaCard.kt:313）；删除二次确认（destructive、**点遮罩不关闭**，
 *   必须明确选「取消」或「删除」——OtaCard.kt:457 `dismissOnOutside = false` 口径）。
 *
 * 边界登记（不越界）：固件默认 `/data/firmware` 在 Files root `/data` 内且不受
 * PROTECTED_NAMES 保护——UI 不暗示「固件受 Files 保护」；`/api/ota/firmware/latest`
 * 与 `GET/POST /api/ota` 是设备侧契约，本页 0 调用；无「管理设备」类跳转。
 */

import { useCallback, useEffect, useRef, useState, type ReactElement } from "react";
import { LoaderCircle } from "lucide-react";
import { SectionCard } from "@/components/config/SectionCard";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { FormNotice } from "@/components/form/FormNotice";
import { useToast } from "@/components/feedback/ToastHost";
import { ApiError } from "@/lib/api";
import {
  deleteFirmware,
  getFirmwareList,
  otaErrorMessage,
  uploadFirmware,
  type FirmwareEntry,
  type OtaFirmwareList,
} from "@/lib/otaApi";

/** 字节 → 人话（B / KB / MB / GB；MB 保留一位小数）。 */
function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) {
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }
  if (bytes >= 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  if (bytes >= 1024) {
    return `${Math.floor(bytes / 1024)} KB`;
  }
  return `${bytes} B`;
}

/** 条目标题（legacy 单列说明；OtaState.kt:287-293 口径）。 */
function entryTitle(entry: FirmwareEntry): string {
  if (entry.legacy) {
    return "旧版单固件（不参与升级）";
  }
  return `${entry.project} · ${entry.model} · ${entry.board === "" ? "该型号通用" : entry.board}`;
}

/** 会拿到这个包的设备（三种，把匹配规则写清楚；OtaState.kt:305-311 逐字）。 */
function matchNote(entry: FirmwareEntry): string {
  if (entry.legacy) {
    return "无型号信息：任何设备都不会拿到它（请按 功能+型号 重新上传）";
  }
  if (entry.board !== "") {
    return `只发给 功能=${entry.project}、型号=${entry.model}、板型=${entry.board} 的设备`;
  }
  return `发给 功能=${entry.project}、型号=${entry.model} 的设备（该型号通用；上传板型专属包可进一步收窄）`;
}

/** 指纹行：IDF v{x} · sha256 前 12 位（缺失项自动省略）。 */
function fingerprint(entry: FirmwareEntry): string {
  const parts: string[] = [];
  if (entry.idfVer !== "") {
    parts.push(`IDF v${entry.idfVer}`);
  }
  if (entry.sha256 !== "") {
    parts.push(`sha256 ${entry.sha256.slice(0, 12)}`);
  }
  return parts.join(" · ");
}

/** 单个覆盖参数输入行（纯 UI state，不是配置字段）。 */
function OverrideField({
  label,
  value,
  placeholder,
  onChange,
}: {
  label: string;
  value: string;
  placeholder: string;
  onChange: (next: string) => void;
}): ReactElement {
  return (
    <div className="space-y-1">
      <Label className="text-sm font-medium">{label}</Label>
      <Input
        className="max-w-md"
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
    </div>
  );
}

export function OtaSection(): ReactElement {
  const { show } = useToast();
  const mountedRef = useRef(true);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  // ---- 清单 ----
  const [list, setList] = useState<OtaFirmwareList | null>(null);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  // ---- 上传（全部是界面本地状态，0 进 draft）----
  const [picked, setPicked] = useState<File | null>(null);
  const [uploading, setUploading] = useState(false);
  const [uploadLoaded, setUploadLoaded] = useState(0);
  const [uploadTotal, setUploadTotal] = useState(0);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [manualOpen, setManualOpen] = useState(false);
  const [overrideProject, setOverrideProject] = useState("");
  const [overrideModel, setOverrideModel] = useState("");
  const [overrideBoard, setOverrideBoard] = useState("");
  const [overrideVersion, setOverrideVersion] = useState("");

  // ---- 删除（destructive 二次确认）----
  const [confirmTarget, setConfirmTarget] = useState<FirmwareEntry | null>(null);
  const [deleting, setDeleting] = useState(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  /** 拉取清单；防重入（请求中禁点）。失败就地 error 行 + 指引，不弹一次性 toast。 */
  const loadList = useCallback(async (): Promise<void> => {
    setLoading(true);
    try {
      const result = await getFirmwareList();
      if (!mountedRef.current) {
        return;
      }
      setList(result);
      setLoadError(null);
      setLoaded(true);
    } catch (error) {
      if (!mountedRef.current) {
        return;
      }
      setList(null);
      setLoadError(otaErrorMessage(error));
      setLoaded(true);
    } finally {
      if (mountedRef.current) {
        setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    void loadList();
  }, [loadList]);

  /** 选包（隐藏 input；取消选择不动已选包）。 */
  const handlePickFile = (): void => {
    fileInputRef.current?.click();
  };

  const handleFileChange = (event: React.ChangeEvent<HTMLInputElement>): void => {
    const file = event.target.files?.[0] ?? null;
    setPicked(file);
    // 同一个文件选择两次也要触发 change：每次打开前清空 input 值。
    event.target.value = "";
  };

  /** 上传：先清上一次失败原因；真实百分比；成功 toast + 自动刷新清单。 */
  const handleUpload = (): void => {
    if (picked === null || uploading) {
      return;
    }
    setUploading(true);
    setUploadError(null);
    setUploadLoaded(0);
    setUploadTotal(picked.size);
    void uploadFirmware(
      picked,
      {
        project: overrideProject,
        model: overrideModel,
        board: overrideBoard,
        version: overrideVersion,
      },
      (loaded, total) => {
        if (!mountedRef.current) {
          return;
        }
        setUploadLoaded(loaded);
        setUploadTotal(total);
      },
    )
      .then((result) => {
        if (!mountedRef.current) {
          return;
        }
        // metadata_from 只按包内是否解析成功判定（手填覆盖时仍可能报 package）——
        // 文案留余地，不写成无条件绝对判定。
        const fromNote =
          result.metadataFrom === "package"
            ? "（服务端标记 metadata_from=package：包内元数据解析成功；若同时手填，手填值优先）"
            : result.metadataFrom === "query"
              ? "（服务端标记 metadata_from=query：包内元数据不可用，使用手填参数）"
              : "";
        show(
          `已上传 ${result.project} · ${result.model} · ${result.version}${fromNote}`,
          "success",
        );
        setPicked(null);
        setUploadError(null);
        void loadList();
      })
      .catch((error: unknown) => {
        if (!mountedRef.current) {
          return;
        }
        if (error instanceof ApiError && error.status === 401) {
          // 401 专项指引（逐字口径）：本页请求会自动注入 Bearer，出现 401 说明
          // 当前会话没有 token（打开页面的 URL 未带 ?token=）。
          setUploadError(
            "上传被拒绝（401）：服务端配置了 server.expected_token。" +
              "网页端请用 http://<host>/?token=<token> 打开本页。" +
              "本页请求会随 URL/会话中的 token 自动注入 Authorization: Bearer；出现 401 即当前会话没有 token。",
          );
        } else {
          setUploadError(otaErrorMessage(error));
        }
      })
      .finally(() => {
        if (mountedRef.current) {
          setUploading(false);
        }
      });
  };

  /** 删除（二次确认后）：成功 toast + 自动刷新；失败 toast（旧端同口径）。 */
  const handleDelete = (target: FirmwareEntry): void => {
    setDeleting(true);
    void deleteFirmware(target.project, target.model, target.board)
      .then(() => {
        if (!mountedRef.current) {
          return;
        }
        show(`已删除 ${target.project} · ${target.model} · ${target.version}`, "success");
        void loadList();
      })
      .catch((error: unknown) => {
        if (!mountedRef.current) {
          return;
        }
        show(otaErrorMessage(error), "error");
      })
      .finally(() => {
        if (mountedRef.current) {
          setDeleting(false);
        }
      });
  };

  // 摘要（distinct 去重；空清单按旧端口径）。
  const summary =
    list === null
      ? null
      : list.entries.length === 0
        ? "尚未托管任何固件"
        : `共 ${list.entries.length} 个固件 · ${new Set(list.entries.map((e) => e.model)).size} 种型号 · ${new Set(list.entries.map((e) => e.project)).size} 个功能`;

  const uploadPct =
    uploadTotal > 0 ? Math.min(100, Math.round((uploadLoaded / uploadTotal) * 100)) : 0;

  return (
    <div className="space-y-4">
      {/* 前言卡：解耦声明逐字保留（ConfigOtaEntryCard.kt:40-47）。 */}
      <SectionCard title="固件与 OTA">
        <div className="space-y-2 text-xs leading-relaxed text-muted-foreground">
          <p>
            固件包托管走独立接口（GET /api/ota/firmware），不随 POST /api/config 回传：保存配置不会动固件，上传/删除固件也不会动配置。
          </p>
          <p>
            上传与删除就在本页下方（唯一一套链路）：设备 OTA 时按「功能 + 型号」匹配，发错型号的包会直接变砖，所以删除必须二次确认。
          </p>
        </div>
      </SectionCard>

      {/* 卡 1：规则说明 + 摘要 + 刷新 + 存储目录。 */}
      <SectionCard
        title="OTA 固件托管"
        description="服务端按「功能 + 型号」二维托管；本卡只读，不含任何配置字段。"
      >
        <div className="space-y-3">
          <p className="text-xs leading-relaxed text-muted-foreground">
            服务端按「功能 + 型号」二维托管固件：设备 OTA 时上报 application.name（功能）、chip_model_name（型号）、board.type（板型）；三者命中且版本更新才会下发下载地址。不同型号 / 不同功能的包绝不互发。
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <p className="text-sm font-medium">{summary ?? (loading ? "加载中…" : "—")}</p>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={loading}
              onClick={() => void loadList()}
            >
              {loading ? <LoaderCircle aria-hidden="true" className="size-4 animate-spin" /> : null}
              {loading ? "刷新中…" : "刷新清单"}
            </Button>
          </div>
          {loadError !== null ? (
            <FormNotice tone="danger">
              {loadError}（确认服务端地址可达后点「刷新清单」）
            </FormNotice>
          ) : null}
          {list !== null && list.dir !== "" ? (
            <p className="text-xs text-muted-foreground">存储目录：{list.dir}</p>
          ) : null}
        </div>
      </SectionCard>

      {/* 卡 2：上传（选包 + 真实进度 + 手动覆盖默认收起 + 常驻失败 Alert）。 */}
      <SectionCard title="上传固件包">
        <div className="space-y-3">
          <p className="text-xs leading-relaxed text-muted-foreground">
            选择 idf.py build 产出的 app .bin：功能 / 型号 / 版本由包内元数据自动识别（ESP-IDF 镜像头的 chip_id + esp_app_desc_t），无需手填。同 功能+型号 再次上传即覆盖旧包。
          </p>

          <input
            ref={fileInputRef}
            type="file"
            accept=".bin,application/octet-stream"
            className="hidden"
            onChange={handleFileChange}
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={uploading}
              onClick={handlePickFile}
            >
              选择 .bin 文件
            </Button>
            <Button
              type="button"
              variant="default"
              size="sm"
              disabled={picked === null || uploading}
              onClick={handleUpload}
            >
              {uploading ? <LoaderCircle aria-hidden="true" className="size-4 animate-spin" /> : null}
              上传固件
            </Button>
            {picked !== null ? (
              <span className="text-sm font-medium text-primary">
                已选：{picked.name}（{formatBytes(picked.size)}）
              </span>
            ) : null}
          </div>

          {uploading ? (
            <div className="space-y-1">
              {/* 真实百分比进度（XHR upload.onprogress），不显示假数字。 */}
              <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full rounded-full bg-primary transition-all"
                  style={{ width: `${uploadPct}%` }}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                上传中：{picked?.name ?? "固件包"}（{picked !== null ? formatBytes(picked.size) : ""}）· {uploadPct}%
              </p>
            </div>
          ) : null}

          {uploadError !== null ? (
            <FormNotice tone="danger">
              <p className="font-medium">上传失败</p>
              <p className="mt-1 break-words">{uploadError}</p>
            </FormNotice>
          ) : null}

          {/* 手动覆盖默认收起：正常路径（ESP-IDF 包）用不到。4 个字段是 URL query
              覆盖参数，不是配置字段——本地 state，不进 draft。 */}
          <div className="flex items-center gap-2 border-t border-border pt-3">
            <Switch
              aria-label="手动指定功能 / 型号 / 版本"
              checked={manualOpen}
              onCheckedChange={setManualOpen}
            />
            <div>
              <p className="text-sm font-medium">手动指定功能 / 型号 / 版本（默认自动识别）</p>
              <p className="text-xs text-muted-foreground">
                只有非 ESP-IDF 镜像、或包内元数据缺字段时才需要手填；留空即由服务端自动识别。
              </p>
            </div>
          </div>
          {manualOpen ? (
            <div className="grid gap-3">
              <OverrideField
                label="功能（project）"
                value={overrideProject}
                placeholder="如 xiaozhi"
                onChange={setOverrideProject}
              />
              <OverrideField
                label="型号（model）"
                value={overrideModel}
                placeholder="如 esp32c3 / esp32s3"
                onChange={setOverrideModel}
              />
              <OverrideField
                label="板型（board，可选）"
                value={overrideBoard}
                placeholder="如 bread-compact-wifi；留空 = 该型号通用"
                onChange={setOverrideBoard}
              />
              <OverrideField
                label="版本（version，可选）"
                value={overrideVersion}
                placeholder="如 2.0.4；留空 = 用包内版本"
                onChange={setOverrideVersion}
              />
            </div>
          ) : null}
        </div>
      </SectionCard>

      {/* 卡 3：已托管清单（固件列含 matchNote + 指纹；大小/编译时间右对齐）。 */}
      <SectionCard title="已托管固件">
        <div className="space-y-3">
          {loaded && loadError === null && (list === null || list.entries.length === 0) ? (
            <p className="text-xs leading-relaxed text-muted-foreground">
              尚未托管任何固件：上传后，设备下次 OTA 检查（开机 / 重连）即可拿到升级。
            </p>
          ) : null}
          {list !== null && list.entries.length > 0 ? (
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-border text-left text-xs text-muted-foreground">
                    <th className="py-2 pr-3 font-medium">固件</th>
                    <th className="py-2 pr-3 font-medium">版本</th>
                    <th className="py-2 pr-3 text-right font-medium">大小</th>
                    <th className="py-2 pr-3 text-right font-medium">编译时间</th>
                    <th className="py-2 text-right font-medium">操作</th>
                  </tr>
                </thead>
                <tbody>
                  {list.entries.map((entry) => (
                    <tr key={`${entry.project}|${entry.model}|${entry.board}`} className="border-b border-border/60 align-top">
                      <td className="max-w-md py-2 pr-3">
                        <p className="font-medium">{entryTitle(entry)}</p>
                        <p
                          className={
                            entry.legacy
                              ? "mt-0.5 text-xs text-amber-700"
                              : "mt-0.5 text-xs text-muted-foreground"
                          }
                        >
                          {matchNote(entry)}
                        </p>
                        {fingerprint(entry) !== "" ? (
                          <p className="mt-0.5 truncate text-xs text-muted-foreground">
                            {fingerprint(entry)}
                          </p>
                        ) : null}
                      </td>
                      <td className="py-2 pr-3">{entry.version === "" ? "—" : entry.version}</td>
                      <td className="py-2 pr-3 text-right">{formatBytes(entry.size)}</td>
                      <td className="py-2 pr-3 text-right">
                        {entry.builtAt === "" ? "—" : entry.builtAt}
                      </td>
                      <td className="py-2 text-right">
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          disabled={deleting}
                          onClick={() => setConfirmTarget(entry)}
                        >
                          删除
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : null}
          {list !== null && list.entries.length > 0 ? (
            <p className="text-xs text-muted-foreground">
              大小 / 编译时间右对齐便于纵向比对；「删除」需二次确认，删除后该 功能+型号 的设备不再收到这条升级（已升级到该版本的设备不受影响）。
            </p>
          ) : null}
        </div>
      </SectionCard>

      {/* 删除二次确认：destructive；点遮罩不关闭（onOpenChange 只接受 open=true 之外的
          关闭请求一律忽略），必须明确点「取消」或「删除」。 */}
      <AlertDialog
        open={confirmTarget !== null}
        onOpenChange={(next) => {
          if (next) {
            return;
          }
          // 点遮罩 / ESC 的关闭请求一律忽略：删除不可撤销，必须明确选「取消」或「删除」。
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>删除固件确认</AlertDialogTitle>
            <AlertDialogDescription>
              {confirmTarget !== null
                ? `确定删除「${entryTitle(confirmTarget)}」吗？删除后，该 功能+型号 的设备将不再收到这条升级；已升级到该版本的设备不受影响。此操作不可撤销。`
                : ""}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={() => setConfirmTarget(null)}>取消</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={deleting}
              onClick={() => {
                const target = confirmTarget;
                setConfirmTarget(null);
                if (target !== null) {
                  handleDelete(target);
                }
              }}
            >
              {deleting ? <LoaderCircle aria-hidden="true" className="size-4 animate-spin" /> : null}
              删除
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
