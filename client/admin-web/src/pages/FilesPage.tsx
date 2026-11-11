import { type ChangeEvent, type FormEvent, type ReactElement, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { ApiError } from "@/lib/api";
import { formatDateTime } from "@/lib/utils";
import { useApiQuery } from "@/lib/useApiQuery";
import {
  deletePath,
  downloadUrl,
  getFileList,
  makeDir,
  uploadFile,
  type FileEntry,
} from "@/lib/adminApi";

/** 服务端 `MAX_FILE_BYTES` 的同口径上限（64MB），用于在浏览器侧提前拦截。 */
const MAX_UPLOAD_BYTES = 64 * 1024 * 1024;

/** 拼接目录与条目名；空目录视为存储根。 */
function joinPath(dir: string, name: string): string {
  return dir === "" ? name : `${dir}/${name}`;
}

/** 取上一级目录；已在根目录时返回空串。 */
function parentPath(dir: string): string {
  const index = dir.lastIndexOf("/");
  return index <= 0 ? "" : dir.slice(0, index);
}

/** 把字节数格式化为 B / KB / MB / GB；0 字节显示「0 B」而不是「—」。 */
function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "—";
  }
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${value} ${units[unit]}` : `${value.toFixed(1)} ${units[unit]}`;
}

/** 服务端 400 的响应体是 `{"error":"<可读原因>"}`，优先透出原因为用户提示。 */
function serverErrorMessage(body: string): string | null {
  if (body.trim() === "") {
    return null;
  }
  try {
    const parsed = JSON.parse(body) as { error?: unknown; message?: unknown };
    if (typeof parsed.error === "string" && parsed.error !== "") {
      return parsed.error;
    }
    if (typeof parsed.message === "string" && parsed.message !== "") {
      return parsed.message;
    }
  } catch {
    // 纯文本响应：直接返回原文，比只显示 HTTP 状态码更有用。
    return body.trim();
  }
  return null;
}

/**
 * 把文件接口错误翻译成可读文案：403 受保护路径、409 目标已存在、
 * 413 超出 64MB 上限，其余优先透出服务端 400 的具体原因。
 */
function describeError(err: unknown): string {
  if (err instanceof ApiError) {
    switch (err.status) {
      case 403:
        return "该路径受保护（如 data.db 及其附属文件），不允许新建或删除";
      case 404:
        return "路径不存在，可能已被其他会话删除";
      case 409:
        return "目标已存在";
      case 413:
        return "文件超过 64MB 上限，服务端已拒绝接收";
      default: {
        const detail = serverErrorMessage(err.body);
        return detail ?? `操作失败（HTTP ${err.status}）`;
      }
    }
  }
  return err instanceof Error ? err.message : String(err);
}

/*
 * 排序交给服务端：`op_list` 已经按「目录在前、名称字节序」排好，
 * 客户端再按 locale 排会与其它客户端（含 Kuikly 版）顺序不一致，故不再重排。
 */

/** 单行条目：目录可点入、文件可下载，受保护条目禁用删除。 */
function FileRow({
  entry,
  fullPath,
  onOpen,
  onDelete,
  disabled,
}: {
  entry: FileEntry;
  fullPath: string;
  onOpen: () => void;
  onDelete: () => void;
  disabled: boolean;
}) {
  return (
    <div className="flex flex-wrap items-center gap-3 border-b px-4 py-2 last:border-b-0 hover:bg-accent/40">
      <div className="flex min-w-0 flex-1 items-center gap-2">
        {entry.dir ? (
          <button
            type="button"
            onClick={onOpen}
            className="truncate font-mono text-sm font-medium hover:underline"
            title={entry.name}
          >
            📁 {entry.name}/
          </button>
        ) : (
          <span className="truncate font-mono text-sm" title={entry.name}>
            {entry.name}
          </span>
        )}
        {entry.protected ? (
          <Badge
            variant="outline"
            className="border-amber-500/40 text-amber-700"
            title="data.db 及其附属文件由服务端保护，不可删除或覆盖"
          >
            受保护
          </Badge>
        ) : null}
      </div>
      <span className="w-24 shrink-0 text-right text-xs text-muted-foreground">
        {entry.dir ? "目录" : formatBytes(entry.size)}
      </span>
      <span className="w-44 shrink-0 text-right text-xs text-muted-foreground">
        {/* 服务端 modified 是 Unix 秒，formatDateTime 需要毫秒。 */}
        {formatDateTime(entry.modified > 0 ? entry.modified * 1000 : null)}
      </span>
      <div className="flex shrink-0 items-center gap-1">
        {entry.dir ? null : (
          <Button variant="ghost" size="xs" asChild>
            <a href={downloadUrl(fullPath)} download={entry.name}>
              下载
            </a>
          </Button>
        )}
        <Button
          variant="ghost"
          size="xs"
          className="text-destructive hover:text-destructive"
          onClick={onDelete}
          disabled={disabled || entry.protected}
          title={entry.protected ? "受保护路径不允许删除" : "删除（目录将递归删除）"}
        >
          删除
        </Button>
      </div>
    </div>
  );
}

export function FilesPage(): ReactElement {
  const [cwd, setCwd] = useState("");
  const [dirName, setDirName] = useState("");
  const [busy, setBusy] = useState(false);
  const [uploading, setUploading] = useState<{
    name: string;
    percent: number;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const listing = useApiQuery(() => getFileList(cwd), [cwd]);
  const entries: FileEntry[] = listing.data?.entries ?? [];
  const segments = cwd === "" ? [] : cwd.split("/");
  const displayPath = listing.data?.path ?? cwd;

  const runMutation = (task: () => Promise<void>): void => {
    setBusy(true);
    setError(null);
    setNotice(null);
    task()
      .catch((err: unknown) => {
        setError(describeError(err));
      })
      .finally(() => {
        setBusy(false);
      });
  };

  const handleMkdir = (event: FormEvent): void => {
    event.preventDefault();
    const name = dirName.trim();
    if (name === "") {
      return;
    }
    const target = joinPath(cwd, name);
    runMutation(async () => {
      await makeDir(target);
      setNotice(`已创建目录：${target}`);
      setDirName("");
      await listing.refetch();
    });
  };

  const handleDelete = (entry: FileEntry): void => {
    const target = joinPath(cwd, entry.name);
    const label = entry.dir ? `目录「${entry.name}」及其全部内容` : `文件「${entry.name}」`;
    if (!window.confirm(`确认删除${label}？此操作不可撤销。`)) {
      return;
    }
    runMutation(async () => {
      await deletePath(target);
      setNotice(`已删除：${target}`);
      await listing.refetch();
    });
  };

  const handleUpload = (file: File): void => {
    if (file.size > MAX_UPLOAD_BYTES) {
      setError(`「${file.name}」${formatBytes(file.size)} 超过 64MB 上限，未发起上传`);
      return;
    }
    const target = joinPath(cwd, file.name);
    const doUpload = (overwrite: boolean) =>
      uploadFile(target, file, {
        overwrite,
        onProgress: (loaded, total) => {
          setUploading({
            name: file.name,
            percent: total > 0 ? Math.floor((loaded * 100) / total) : 0,
          });
        },
      });

    setBusy(true);
    setError(null);
    setNotice(null);
    setUploading({ name: file.name, percent: 0 });
    doUpload(false)
      .then(async (res) => {
        if (!res.ok) {
          setError(res.message ?? "上传未成功");
          return;
        }
        setNotice(`已上传：${target}（${formatBytes(file.size)}）`);
        await listing.refetch();
      })
      .catch(async (err: unknown) => {
        // 409 表示目标已存在：二次确认后用 overwrite=1 重试一次。
        if (err instanceof ApiError && err.status === 409) {
          if (!window.confirm(`「${file.name}」已存在，是否覆盖？`)) {
            setNotice("已取消覆盖上传");
            return;
          }
          const res = await doUpload(true);
          if (!res.ok) {
            setError(res.message ?? "上传未成功");
            return;
          }
          setNotice(`已覆盖上传：${target}（${formatBytes(file.size)}）`);
          await listing.refetch();
          return;
        }
        throw err;
      })
      .catch((err: unknown) => {
        setError(describeError(err));
      })
      .finally(() => {
        setBusy(false);
        setUploading(null);
      });
  };

  const handleFileChange = (event: ChangeEvent<HTMLInputElement>): void => {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (file) {
      handleUpload(file);
    }
  };

  const handleRefresh = (): void => {
    setError(null);
    setNotice(null);
    listing.refetch();
  };

  return (
    <div className="mx-auto w-full max-w-6xl space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="space-y-1">
          <h2 className="text-2xl font-semibold tracking-tight">文件管理</h2>
          <p className="text-sm text-muted-foreground">
            固件、音频与静态资源的上传、下载和删除；单文件上限 64MB。
          </p>
        </div>
        <div className="flex items-center gap-3">
          <span className="text-xs text-muted-foreground">
            最后更新：{formatDateTime(listing.updatedAt)}
          </span>
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            disabled={listing.loading || busy}
          >
            {listing.loading ? "刷新中…" : "刷新"}
          </Button>
        </div>
      </div>
      <Separator />

      {error !== null ? (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {error}
        </div>
      ) : null}
      {notice !== null ? (
        <div className="rounded-md border border-emerald-500/40 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-700">
          {notice}
        </div>
      ) : null}

      <Card>
        <CardHeader className="pb-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="space-y-1">
              <CardTitle className="text-base">当前位置</CardTitle>
              <CardDescription className="font-mono text-xs">
                {displayPath === "" ? "存储根" : displayPath}
              </CardDescription>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => setCwd(parentPath(cwd))}
                disabled={cwd === "" || busy}
              >
                上级
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => fileInputRef.current?.click()}
                disabled={busy}
              >
                上传文件
              </Button>
              <input
                ref={fileInputRef}
                type="file"
                className="hidden"
                onChange={handleFileChange}
              />
            </div>
          </div>
          <nav className="flex flex-wrap items-center gap-1 pt-2 text-sm">
            <button
              type="button"
              onClick={() => setCwd("")}
              className="text-muted-foreground hover:text-foreground hover:underline"
            >
              根目录
            </button>
            {segments.map((segment, index) => {
              const target = segments.slice(0, index + 1).join("/");
              const isLast = index === segments.length - 1;
              return (
                <span key={target} className="flex items-center gap-1">
                  <span className="text-muted-foreground">/</span>
                  {isLast ? (
                    <span className="font-medium">{segment}</span>
                  ) : (
                    <button
                      type="button"
                      onClick={() => setCwd(target)}
                      className="text-muted-foreground hover:text-foreground hover:underline"
                    >
                      {segment}
                    </button>
                  )}
                </span>
              );
            })}
          </nav>
        </CardHeader>
        <CardContent className="space-y-3">
          <form onSubmit={handleMkdir} className="flex flex-wrap items-center gap-2">
            <input
              value={dirName}
              onChange={(event) => setDirName(event.target.value)}
              placeholder="新建目录名称（支持 a/b 多级）"
              className="min-w-[240px] flex-1 rounded-md border px-2 py-1 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
            />
            <Button type="submit" size="sm" disabled={busy || dirName.trim() === ""}>
              新建目录
            </Button>
          </form>
          {uploading !== null ? (
            <p className="text-xs text-muted-foreground">
              正在上传「{uploading.name}」：{uploading.percent}%
            </p>
          ) : null}

          <div className="overflow-hidden rounded-md border">
            <div className="flex items-center gap-3 border-b bg-muted/40 px-4 py-2 text-xs font-medium text-muted-foreground">
              <span className="flex-1">名称</span>
              <span className="w-24 text-right">大小</span>
              <span className="w-44 text-right">修改时间</span>
              <span className="w-[104px] text-right">操作</span>
            </div>
            {listing.error !== null ? (
              // 列表请求失败必须显式告知：此前该分支缺失，接口 401/500 时会
              // 落到「该目录为空」，让用户误以为文件被删光。
              <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-4 text-sm text-destructive">
                <span>目录加载失败：{listing.error}</span>
                <Button variant="outline" size="sm" onClick={handleRefresh}>
                  重试
                </Button>
              </div>
            ) : listing.loading ? (
              <div className="px-4 py-6 text-sm text-muted-foreground">加载中…</div>
            ) : entries.length === 0 ? (
              <div className="px-4 py-6 text-sm text-muted-foreground">该目录为空</div>
            ) : (
              entries.map((entry) => (
                <FileRow
                  key={entry.name}
                  entry={entry}
                  fullPath={joinPath(cwd, entry.name)}
                  disabled={busy}
                  onOpen={() => setCwd(joinPath(cwd, entry.name))}
                  onDelete={() => handleDelete(entry)}
                />
              ))
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
