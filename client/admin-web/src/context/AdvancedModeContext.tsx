/**
 * 「高级模式」开关：控制配置页里高级字段是否展开。
 *
 * 默认关闭；用户选择持久化到 localStorage（key `admin.config.advancedMode`），
 * 下次打开管理台仍保持。localStorage 在隐私模式/禁用时写入会抛错，这里静默忽略，
 * 仅退化为「本次会话有效」。
 */

import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactElement,
  type ReactNode,
} from "react";

const STORAGE_KEY = "admin.config.advancedMode";

export interface AdvancedModeValue {
  advancedMode: boolean;
  toggle: () => void;
}

const AdvancedModeContext = createContext<AdvancedModeValue | null>(null);

/** 读取初始值；任何异常都按“关闭”处理。 */
function readInitialMode(): boolean {
  if (typeof window === "undefined") {
    return false;
  }
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

/** 为子树提供高级模式状态。 */
export function AdvancedModeProvider({ children }: { children: ReactNode }): ReactElement {
  const [advancedMode, setAdvancedMode] = useState<boolean>(readInitialMode);

  const toggle = useCallback((): void => {
    setAdvancedMode((previous) => {
      const next = !previous;
      try {
        window.localStorage.setItem(STORAGE_KEY, String(next));
      } catch {
        // 忽略：存储不可用时只在当前会话生效。
      }
      return next;
    });
  }, []);

  const value = useMemo<AdvancedModeValue>(
    () => ({ advancedMode, toggle }),
    [advancedMode, toggle],
  );

  return (
    <AdvancedModeContext.Provider value={value}>{children}</AdvancedModeContext.Provider>
  );
}

/** 读取高级模式；必须在 `AdvancedModeProvider` 内使用。 */
export function useAdvancedMode(): AdvancedModeValue {
  const value = useContext(AdvancedModeContext);
  if (value === null) {
    throw new Error("useAdvancedMode 必须在 AdvancedModeProvider 内使用");
  }
  return value;
}
