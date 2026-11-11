import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** 合并条件类名并消除 Tailwind 冲突（shadcn/ui cn() 标准实现）。 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}

/** 把毫秒时长格式化为「N天 N小时 N分」（不足部分省略，0 则显示「0分」）。 */
export function formatUptime(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) {
    return "—";
  }
  const totalMinutes = Math.floor(ms / 60000);
  const days = Math.floor(totalMinutes / 1440);
  const hours = Math.floor((totalMinutes % 1440) / 60);
  const minutes = totalMinutes % 60;
  const parts: string[] = [];
  if (days > 0) {
    parts.push(`${days}天`);
  }
  if (hours > 0) {
    parts.push(`${hours}小时`);
  }
  if (minutes > 0 || parts.length === 0) {
    parts.push(`${minutes}分`);
  }
  return parts.join(" ");
}

/** 把绝对时间戳格式化为「刚刚 / N分钟前 / N小时前 / N天前」。 */
export function formatRelative(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) {
    return "—";
  }
  const diff = Date.now() - ms;
  if (diff < 0) {
    return "刚刚";
  }
  const minutes = Math.floor(diff / 60000);
  if (minutes < 1) {
    return "刚刚";
  }
  if (minutes < 60) {
    return `${minutes}分钟前`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours}小时前`;
  }
  const days = Math.floor(hours / 24);
  return `${days}天前`;
}

/** 把时间戳格式化为本地可读时间字符串；null 或非法值返回「—」。 */
export function formatDateTime(ts: number | null): string {
  if (ts === null || !Number.isFinite(ts)) {
    return "—";
  }
  return new Date(ts).toLocaleString();
}
