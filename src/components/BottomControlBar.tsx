import { useEffect, useState } from "react";
import { Activity, Lock, Settings, Terminal } from "lucide-react";
import type { SystemToolKind } from "../types/dock";
import { Tooltip } from "./Tooltip";

interface BottomControlBarProps {
  expanded: boolean;
  hasHotkeyWarning: boolean;
  onSystemTool: (tool: SystemToolKind) => void;
  onOpenSettings: () => void;
}

function formatClock(now: Date): string {
  const hours = String(now.getHours()).padStart(2, "0");
  const minutes = String(now.getMinutes()).padStart(2, "0");
  return `${hours}:${minutes}`;
}

function formatDate(now: Date): string {
  return new Intl.DateTimeFormat("en-US", {
    weekday: "short",
    month: "short",
    day: "numeric",
  }).format(now);
}

export function BottomControlBar({
  expanded,
  hasHotkeyWarning,
  onSystemTool,
  onOpenSettings,
}: BottomControlBarProps) {
  const [now, setNow] = useState<Date>(() => new Date());

  // Pause clock ticking whenever the dock is collapsed to conserve idle CPU.
  useEffect(() => {
    if (!expanded) {
      return;
    }
    setNow(new Date());
    const timer = window.setInterval(() => {
      setNow(new Date());
    }, 1000);
    return () => {
      window.clearInterval(timer);
    };
  }, [expanded]);

  return (
    <footer className="h-[52px] px-3 bg-harbor-controls border-t border-harbor-border flex items-center justify-between shrink-0">
      <div className="flex items-center gap-2.5 min-w-0">
        <span
          className="w-2.5 h-2.5 rounded-full bg-harbor-success shrink-0"
          aria-hidden="true"
        />
        <div className="leading-tight min-w-0">
          <div className="text-[13px] font-semibold text-harbor-textPrimary tabular-nums">
            {formatClock(now)}
          </div>
          <div className="text-[11px] text-harbor-textMuted truncate">
            {formatDate(now)}
          </div>
        </div>
      </div>

      <div className="flex items-center gap-1 shrink-0">
        <Tooltip label="Task Manager" side="top">
          <button
            type="button"
            aria-label="Open Task Manager"
            onClick={() => onSystemTool("taskmgr")}
            className="w-8 h-8 rounded-md flex items-center justify-center text-harbor-textSecondary hover:text-harbor-textPrimary hover:bg-harbor-hover transition-colors"
          >
            <Activity className="w-4 h-4" strokeWidth={1.75} />
          </button>
        </Tooltip>

        <Tooltip label="Terminal" side="top">
          <button
            type="button"
            aria-label="Open Terminal"
            onClick={() => onSystemTool("terminal")}
            className="w-8 h-8 rounded-md flex items-center justify-center text-harbor-textSecondary hover:text-harbor-textPrimary hover:bg-harbor-hover transition-colors"
          >
            <Terminal className="w-4 h-4" strokeWidth={1.75} />
          </button>
        </Tooltip>

        <Tooltip label="Lock PC" side="top">
          <button
            type="button"
            aria-label="Lock PC"
            onClick={() => onSystemTool("lock")}
            className="w-8 h-8 rounded-md flex items-center justify-center text-harbor-textSecondary hover:text-harbor-textPrimary hover:bg-harbor-hover transition-colors"
          >
            <Lock className="w-4 h-4" strokeWidth={1.75} />
          </button>
        </Tooltip>

        <Tooltip label="Settings" side="top">
          <button
            type="button"
            aria-label="Open Settings"
            onClick={onOpenSettings}
            className="relative w-8 h-8 rounded-md flex items-center justify-center text-harbor-textSecondary hover:text-harbor-textPrimary hover:bg-harbor-hover transition-colors"
          >
            <Settings className="w-4 h-4" strokeWidth={1.75} />
            {hasHotkeyWarning && (
              <span
                className="absolute top-1.5 right-1.5 w-2 h-2 rounded-full bg-harbor-warning"
                aria-hidden="true"
              />
            )}
          </button>
        </Tooltip>
      </div>
    </footer>
  );
}
