/**
 * 受控确认框：基于 shadcn 的 alert-dialog。
 *
 * 取消路径（取消按钮 / ESC / 点遮罩）统一走 `onCancel`；确认路径只走 `onConfirm`，
 * 用 ref 避免 Radix 自动关闭时把确认误报成取消。
 */

import { useEffect, useRef, type ReactElement } from "react";
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

export interface ConfirmDialogProps {
  open: boolean;
  title: string;
  description?: string;
  confirmText?: string;
  destructive?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  open,
  title,
  description,
  confirmText = "确认",
  destructive = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps): ReactElement {
  const confirmedRef = useRef(false);

  useEffect(() => {
    if (open) {
      confirmedRef.current = false;
    }
  }, [open]);

  const handleOpenChange = (next: boolean): void => {
    if (next) {
      return;
    }
    if (confirmedRef.current) {
      // 确认后 Radix 自动关闭：只重置标记，不再触发 onCancel。
      confirmedRef.current = false;
      return;
    }
    onCancel();
  };

  const handleConfirm = (): void => {
    confirmedRef.current = true;
    onConfirm();
  };

  return (
    <AlertDialog open={open} onOpenChange={handleOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          {description ? (
            <AlertDialogDescription>{description}</AlertDialogDescription>
          ) : (
            <AlertDialogDescription className="sr-only">
              请确认是否继续此操作。
            </AlertDialogDescription>
          )}
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>取消</AlertDialogCancel>
          <AlertDialogAction
            variant={destructive ? "destructive" : "default"}
            onClick={handleConfirm}
          >
            {confirmText}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
