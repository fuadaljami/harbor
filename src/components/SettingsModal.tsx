import { useEffect, useState } from "react";
import {
  AlertTriangle,
  HardDrive,
  Keyboard,
  Monitor,
  RefreshCw,
  RotateCcw,
  X,
} from "lucide-react";
import { ipc, parseAppError } from "../lib/ipc";
import type {
  DockConfig,
  DockEdge,
  MonitorDescriptor,
  SettingsUpdate,
} from "../types/dock";

interface SettingsModalProps {
  isOpen: boolean;
  config: DockConfig;
  monitors: MonitorDescriptor[];
  hotkeyWarning: string | null;
  isPortable: boolean;
  isRescanning: boolean;
  onClose: () => void;
  onSaveSettings: (settings: SettingsUpdate) => Promise<void>;
  onRescan: () => Promise<string[]>;
  onDismissMissing: (ids: string[]) => Promise<void>;
  onResetDefaults: () => Promise<void>;
}

function formatHotkeyEvent(e: React.KeyboardEvent<HTMLInputElement>): string | null {
  const ignoredKeys = new Set(["Control", "Shift", "Alt", "Meta"]);
  if (ignoredKeys.has(e.key)) {
    return null;
  }

  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Super");

  if (parts.length === 0) {
    return null;
  }

  let mainKey = e.key;
  if (mainKey === " ") {
    mainKey = "Space";
  } else if (mainKey.length === 1) {
    mainKey = mainKey.toUpperCase();
  }
  parts.push(mainKey);
  return parts.join("+");
}

export function SettingsModal({
  isOpen,
  config,
  monitors,
  hotkeyWarning,
  isPortable,
  isRescanning,
  onClose,
  onSaveSettings,
  onRescan,
  onDismissMissing,
  onResetDefaults,
}: SettingsModalProps) {
  const [edge, setEdge] = useState<DockEdge>(config.edge);
  const [monitorName, setMonitorName] = useState<string>(
    config.monitor_name ?? "",
  );
  const [globalHotkey, setGlobalHotkey] = useState<string>(config.global_hotkey);
  const [recordingHotkey, setRecordingHotkey] = useState(false);
  const [autoCollapseOnLaunch, setAutoCollapseOnLaunch] = useState(
    config.auto_collapse_on_launch,
  );
  const [autoCollapseOnBlur, setAutoCollapseOnBlur] = useState(
    config.auto_collapse_on_blur,
  );
  const [alwaysOnTop, setAlwaysOnTop] = useState(config.always_on_top);
  const [startWithWindows, setStartWithWindows] = useState(
    config.start_with_windows,
  );
  const [liveMonitors, setLiveMonitors] = useState<MonitorDescriptor[]>(monitors);
  const [missingIds, setMissingIds] = useState<string[]>([]);
  const [confirmReset, setConfirmReset] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!isOpen) return;
    setEdge(config.edge);
    setMonitorName(config.monitor_name ?? "");
    setGlobalHotkey(config.global_hotkey);
    setRecordingHotkey(false);
    setAutoCollapseOnLaunch(config.auto_collapse_on_launch);
    setAutoCollapseOnBlur(config.auto_collapse_on_blur);
    setAlwaysOnTop(config.always_on_top);
    setStartWithWindows(config.start_with_windows);
    setLiveMonitors(monitors);
    setMissingIds([]);
    setConfirmReset(false);
    setErrorMsg(null);
    setSaving(false);

    void ipc
      .listMonitors()
      .then((list) => setLiveMonitors(list))
      .catch(() => {});
  }, [isOpen, config, monitors]);

  useEffect(() => {
    if (!isOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !recordingHotkey) {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [isOpen, recordingHotkey, onClose]);

  if (!isOpen) {
    return null;
  }

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    const cleanedHotkey = globalHotkey.trim();
    if (!cleanedHotkey) {
      setErrorMsg("Global hotkey cannot be empty.");
      return;
    }

    setSaving(true);
    setErrorMsg(null);
    try {
      await onSaveSettings({
        edge,
        monitor_name: monitorName.trim() || null,
        auto_collapse_on_launch: autoCollapseOnLaunch,
        auto_collapse_on_blur: autoCollapseOnBlur,
        always_on_top: alwaysOnTop,
        start_with_windows: startWithWindows,
        global_hotkey: cleanedHotkey,
      });
      onClose();
    } catch (err) {
      setErrorMsg(parseAppError(err).message);
    } finally {
      setSaving(false);
    }
  };

  const handleRescanClick = async () => {
    setErrorMsg(null);
    try {
      const ids = await onRescan();
      setMissingIds(ids);
    } catch (err) {
      setErrorMsg(parseAppError(err).message);
    }
  };

  const handleDismissMissingClick = async () => {
    try {
      await onDismissMissing(missingIds);
      setMissingIds([]);
    } catch (err) {
      setErrorMsg(parseAppError(err).message);
    }
  };

  const handleConfirmReset = async () => {
    setSaving(true);
    setErrorMsg(null);
    try {
      await onResetDefaults();
      setConfirmReset(false);
      setMissingIds([]);
    } catch (err) {
      setErrorMsg(parseAppError(err).message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="absolute inset-0 z-40 bg-black/65 flex items-center justify-center p-2.5"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Harbor Settings"
        className="w-full max-h-full rounded-lg bg-harbor-modal border border-white/[0.08] shadow-2xl flex flex-col overflow-hidden"
      >
        <div className="px-4 py-3 border-b border-harbor-border flex items-center justify-between shrink-0">
          <div className="flex items-center gap-2">
            <h2 className="text-[14px] font-semibold text-harbor-textPrimary">
              Settings
            </h2>
            <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-harbor-rail border border-white/[0.06] text-[10px] font-semibold uppercase tracking-wider text-harbor-textSecondary">
              <HardDrive className="w-3 h-3 text-harbor-blurple" strokeWidth={1.75} />
              {isPortable ? "Portable" : "Installed"}
            </span>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close settings"
            className="p-1 rounded text-harbor-textMuted hover:text-harbor-textPrimary hover:bg-harbor-hover transition-colors"
          >
            <X className="w-4 h-4" strokeWidth={1.75} />
          </button>
        </div>

        <form
          onSubmit={(e) => void handleSave(e)}
          className="flex-1 overflow-y-auto px-4 py-3 flex flex-col gap-3.5 text-xs"
        >
          {hotkeyWarning && (
            <div className="p-2.5 rounded-md bg-harbor-warning/15 border border-harbor-warning/40 flex items-start gap-2 text-harbor-textPrimary">
              <AlertTriangle
                className="w-4 h-4 text-harbor-warning shrink-0 mt-0.5"
                strokeWidth={1.75}
              />
              <span className="text-[11.5px] leading-snug">{hotkeyWarning}</span>
            </div>
          )}

          <div>
            <label
              htmlFor="settings-monitor"
              className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted mb-1"
            >
              <Monitor className="w-3.5 h-3.5" strokeWidth={1.75} />
              <span>Display Monitor</span>
            </label>
            <select
              id="settings-monitor"
              value={monitorName}
              onChange={(e) => setMonitorName(e.target.value)}
              className="w-full px-2.5 py-2 rounded-md bg-harbor-rail border border-white/[0.06] text-[12.5px] text-harbor-textPrimary focus:outline-none focus:border-harbor-blurple"
            >
              <option value="">Primary Display (Auto)</option>
              {liveMonitors.map((m) => (
                <option key={m.name} value={m.name}>
                  {m.label}
                </option>
              ))}
            </select>
          </div>

          <div>
            <span className="block text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted mb-1.5">
              Dock Edge
            </span>
            <div className="grid grid-cols-2 gap-1.5">
              {(["Left", "Right"] as const).map((side) => (
                <button
                  key={side}
                  type="button"
                  onClick={() => setEdge(side)}
                  className={`py-1.5 rounded-md font-medium transition-colors ${
                    edge === side
                      ? "bg-harbor-blurple text-white"
                      : "bg-harbor-rail text-harbor-textSecondary hover:bg-harbor-hover hover:text-harbor-textPrimary"
                  }`}
                >
                  {side} Edge
                </button>
              ))}
            </div>
          </div>

          <div>
            <label
              htmlFor="settings-hotkey"
              className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted mb-1"
            >
              <Keyboard className="w-3.5 h-3.5" strokeWidth={1.75} />
              <span>Global Toggle Hotkey</span>
            </label>
            <div className="flex items-center gap-1.5">
              <input
                id="settings-hotkey"
                type="text"
                value={recordingHotkey ? "Press key combo..." : globalHotkey}
                readOnly={recordingHotkey}
                onChange={(e) => {
                  if (!recordingHotkey) setGlobalHotkey(e.target.value);
                }}
                onKeyDown={(e) => {
                  if (!recordingHotkey) return;
                  e.preventDefault();
                  e.stopPropagation();
                  if (e.key === "Escape") {
                    setRecordingHotkey(false);
                    return;
                  }
                  const combo = formatHotkeyEvent(e);
                  if (combo) {
                    setGlobalHotkey(combo);
                    setRecordingHotkey(false);
                  }
                }}
                className={`flex-1 min-w-0 px-2.5 py-2 rounded-md bg-harbor-rail border text-[12.5px] focus:outline-none ${
                  recordingHotkey
                    ? "border-harbor-blurple text-harbor-blurple"
                    : "border-white/[0.06] text-harbor-textPrimary focus:border-harbor-blurple"
                }`}
              />
              <button
                type="button"
                onClick={() => setRecordingHotkey((prev) => !prev)}
                className={`px-2.5 py-2 rounded-md font-medium shrink-0 transition-colors ${
                  recordingHotkey
                    ? "bg-harbor-blurple text-white"
                    : "bg-harbor-hover hover:bg-harbor-active text-harbor-textPrimary"
                }`}
              >
                {recordingHotkey ? "Cancel" : "Record"}
              </button>
            </div>
          </div>

          <div className="flex flex-col gap-2 pt-0.5">
            <span className="text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted">
              Behavior
            </span>

            {[
              {
                label: "Auto-collapse after launching item",
                checked: autoCollapseOnLaunch,
                onChange: setAutoCollapseOnLaunch,
              },
              {
                label: "Auto-collapse when dock loses focus",
                checked: autoCollapseOnBlur,
                onChange: setAutoCollapseOnBlur,
              },
              {
                label: "Keep dock always on top",
                checked: alwaysOnTop,
                onChange: setAlwaysOnTop,
              },
              {
                label: "Start Harbor with Windows",
                checked: startWithWindows,
                onChange: setStartWithWindows,
              },
            ].map((toggle) => (
              <label
                key={toggle.label}
                className="flex items-center justify-between px-2.5 py-1.5 rounded-md bg-harbor-rail cursor-pointer hover:bg-harbor-hover/70 transition-colors"
              >
                <span className="text-harbor-textSecondary">
                  {toggle.label}
                </span>
                <input
                  type="checkbox"
                  checked={toggle.checked}
                  onChange={(e) => toggle.onChange(e.target.checked)}
                  className="rounded border-white/20 bg-harbor-panel text-harbor-blurple focus:ring-harbor-blurple"
                />
              </label>
            ))}
          </div>

          <div className="flex flex-col gap-2 pt-1 border-t border-harbor-border">
            <span className="text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted">
              Library Maintenance
            </span>

            <div className="flex items-center gap-2">
              <button
                type="button"
                disabled={isRescanning || saving}
                onClick={() => void handleRescanClick()}
                className="flex-1 py-1.5 px-2.5 rounded-md bg-harbor-hover hover:bg-harbor-active text-harbor-textPrimary font-medium flex items-center justify-center gap-1.5 disabled:opacity-60 transition-colors"
              >
                <RefreshCw
                  className={`w-3.5 h-3.5 ${isRescanning ? "animate-spin" : ""}`}
                  strokeWidth={1.75}
                />
                <span>{isRescanning ? "Scanning..." : "Rescan System"}</span>
              </button>

              {!confirmReset ? (
                <button
                  type="button"
                  disabled={saving}
                  onClick={() => setConfirmReset(true)}
                  className="py-1.5 px-2.5 rounded-md bg-harbor-rail hover:bg-harbor-danger/20 hover:text-harbor-danger text-harbor-textSecondary font-medium flex items-center gap-1.5 transition-colors"
                >
                  <RotateCcw className="w-3.5 h-3.5" strokeWidth={1.75} />
                  <span>Reset Defaults</span>
                </button>
              ) : (
                <div className="flex items-center gap-1">
                  <button
                    type="button"
                    disabled={saving}
                    onClick={() => void handleConfirmReset()}
                    className="py-1.5 px-2.5 rounded-md bg-harbor-danger hover:bg-harbor-danger/90 text-white font-medium transition-colors"
                  >
                    Confirm Reset
                  </button>
                  <button
                    type="button"
                    onClick={() => setConfirmReset(false)}
                    className="py-1.5 px-2 rounded-md bg-harbor-hover text-harbor-textPrimary font-medium"
                  >
                    Cancel
                  </button>
                </div>
              )}
            </div>

            {missingIds.length > 0 && (
              <div className="p-2.5 rounded-md bg-harbor-rail border border-harbor-warning/40 flex items-center justify-between gap-2">
                <span className="text-[11.5px] text-harbor-textSecondary">
                  {missingIds.length} missing discovered{" "}
                  {missingIds.length === 1 ? "entry" : "entries"} detected.
                </span>
                <div className="flex items-center gap-1 shrink-0">
                  <button
                    type="button"
                    onClick={() => void handleDismissMissingClick()}
                    className="px-2 py-1 rounded bg-harbor-danger/20 hover:bg-harbor-danger text-harbor-danger hover:text-white text-[11px] font-medium transition-colors"
                  >
                    Remove Missing
                  </button>
                  <button
                    type="button"
                    onClick={() => setMissingIds([])}
                    className="px-2 py-1 rounded bg-harbor-hover text-harbor-textSecondary text-[11px] font-medium"
                  >
                    Keep
                  </button>
                </div>
              </div>
            )}
          </div>

          {errorMsg && (
            <div className="px-2.5 py-2 rounded-md bg-harbor-danger/15 border border-harbor-danger/40 text-xs text-harbor-textPrimary">
              {errorMsg}
            </div>
          )}

          <div className="pt-2 border-t border-harbor-border flex items-center justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              className="px-3 py-1.5 rounded-md bg-harbor-hover hover:bg-harbor-active text-harbor-textPrimary font-medium transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={saving}
              className="px-3.5 py-1.5 rounded-md bg-harbor-blurple hover:bg-harbor-blurpleHover text-white font-medium disabled:opacity-60 transition-colors"
            >
              {saving ? "Saving..." : "Save Settings"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
