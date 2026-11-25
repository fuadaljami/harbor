import { useEffect, useState } from "react";
import { FolderOpen, Shield, X } from "lucide-react";
import { ipc, parseAppError } from "../lib/ipc";
import type {
  Category,
  DockItem,
  NewItemInput,
  UpdateItemInput,
} from "../types/dock";

interface AddEditModalProps {
  isOpen: boolean;
  editingItem: DockItem | null;
  defaultCategory: Category;
  onClose: () => void;
  onSaveNew: (input: NewItemInput) => Promise<void>;
  onSaveEdit: (input: UpdateItemInput) => Promise<void>;
}

const CATEGORIES: readonly Category[] = ["Games", "Apps", "Tools"];

function stemFromPath(filePath: string): string {
  const name = filePath.split(/[\\/]/).pop() ?? "";
  return name.replace(/\.[^.]+$/, "");
}

export function AddEditModal({
  isOpen,
  editingItem,
  defaultCategory,
  onClose,
  onSaveNew,
  onSaveEdit,
}: AddEditModalProps) {
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [argumentsText, setArgumentsText] = useState("");
  const [description, setDescription] = useState("");
  const [category, setCategory] = useState<Category>(defaultCategory);
  const [customIconPath, setCustomIconPath] = useState("");
  const [runAsAdmin, setRunAsAdmin] = useState(false);
  const [pinned, setPinned] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const isSteam = editingItem?.source === "Steam";

  useEffect(() => {
    if (!isOpen) return;
    if (editingItem) {
      setName(editingItem.name);
      setPath(editingItem.path);
      setArgumentsText(editingItem.arguments);
      setDescription(editingItem.description);
      setCategory(editingItem.category);
      setCustomIconPath(editingItem.custom_icon_path ?? "");
      setRunAsAdmin(editingItem.run_as_admin);
      setPinned(editingItem.pinned);
    } else {
      setName("");
      setPath("");
      setArgumentsText("");
      setDescription("");
      setCategory(defaultCategory);
      setCustomIconPath("");
      setRunAsAdmin(false);
      setPinned(false);
    }
    setErrorMsg(null);
    setSubmitting(false);
  }, [isOpen, editingItem, defaultCategory]);

  useEffect(() => {
    if (!isOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [isOpen, onClose]);

  if (!isOpen) {
    return null;
  }

  const handlePickExecutable = async () => {
    try {
      const selected = await ipc.pickExecutableFile();
      if (selected) {
        setPath(selected);
        if (!name.trim()) {
          setName(stemFromPath(selected));
        }
        setErrorMsg(null);
      }
    } catch (err) {
      setErrorMsg(parseAppError(err).message);
    }
  };

  const handlePickIcon = async () => {
    try {
      const selected = await ipc.pickIconFile();
      if (selected) {
        setCustomIconPath(selected);
      }
    } catch (err) {
      setErrorMsg(parseAppError(err).message);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmedName = name.trim();
    const trimmedPath = path.trim();
    if (!trimmedName) {
      setErrorMsg("Display name is required.");
      return;
    }
    if (!isSteam && !trimmedPath) {
      setErrorMsg("Target executable or URI path is required.");
      return;
    }

    setSubmitting(true);
    setErrorMsg(null);
    try {
      if (editingItem) {
        await onSaveEdit({
          id: editingItem.id,
          name: trimmedName,
          path: isSteam ? editingItem.path : trimmedPath,
          arguments: argumentsText.trim(),
          description: description.trim(),
          category,
          custom_icon_path: customIconPath.trim() || null,
          run_as_admin: runAsAdmin,
          pinned,
        });
      } else {
        await onSaveNew({
          name: trimmedName,
          path: trimmedPath,
          arguments: argumentsText.trim(),
          description: description.trim(),
          category,
          custom_icon_path: customIconPath.trim() || null,
          run_as_admin: runAsAdmin,
          pinned,
        });
      }
      onClose();
    } catch (err) {
      setErrorMsg(parseAppError(err).message);
    } finally {
      setSubmitting(false);
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
        aria-label={editingItem ? "Edit Shortcut" : "Add Shortcut"}
        className="w-full max-h-full rounded-lg bg-harbor-modal border border-white/[0.08] shadow-2xl flex flex-col overflow-hidden"
      >
        <div className="px-4 py-3 border-b border-harbor-border flex items-center justify-between shrink-0">
          <h2 className="text-[14px] font-semibold text-harbor-textPrimary">
            {editingItem ? "Edit Shortcut" : "Add Shortcut"}
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close modal"
            className="p-1 rounded text-harbor-textMuted hover:text-harbor-textPrimary hover:bg-harbor-hover transition-colors"
          >
            <X className="w-4 h-4" strokeWidth={1.75} />
          </button>
        </div>

        <form
          onSubmit={(e) => void handleSubmit(e)}
          className="flex-1 overflow-y-auto px-4 py-3 flex flex-col gap-3 text-xs"
        >
          <div>
            <label className="block text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted mb-1.5">
              Category
            </label>
            <div className="grid grid-cols-3 gap-1.5">
              {CATEGORIES.map((cat) => (
                <button
                  key={cat}
                  type="button"
                  onClick={() => setCategory(cat)}
                  className={`py-1.5 rounded-md text-xs font-medium transition-colors ${
                    category === cat
                      ? "bg-harbor-blurple text-white"
                      : "bg-harbor-rail text-harbor-textSecondary hover:bg-harbor-hover hover:text-harbor-textPrimary"
                  }`}
                >
                  {cat}
                </button>
              ))}
            </div>
          </div>

          <div>
            <label
              htmlFor="shortcut-name"
              className="block text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted mb-1"
            >
              Display Name
            </label>
            <input
              id="shortcut-name"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Visual Studio Code"
              className="w-full px-2.5 py-2 rounded-md bg-harbor-rail border border-white/[0.06] text-[13px] text-harbor-textPrimary placeholder:text-harbor-textMuted focus:outline-none focus:border-harbor-blurple"
            />
          </div>

          <div>
            <div className="flex items-center justify-between mb-1">
              <label
                htmlFor="shortcut-path"
                className="text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted"
              >
                Target Path or URI
              </label>
              {isSteam && (
                <span className="text-[10.5px] font-medium text-harbor-blurple">
                  Managed by Steam (read-only)
                </span>
              )}
            </div>
            <div className="flex items-center gap-1.5">
              <input
                id="shortcut-path"
                type="text"
                value={path}
                readOnly={isSteam}
                onChange={(e) => setPath(e.target.value)}
                placeholder="C:\Program Files\App\app.exe"
                className={`flex-1 min-w-0 px-2.5 py-2 rounded-md bg-harbor-rail border border-white/[0.06] text-[12.5px] focus:outline-none ${
                  isSteam
                    ? "text-harbor-textMuted cursor-not-allowed"
                    : "text-harbor-textPrimary focus:border-harbor-blurple"
                }`}
              />
              {!isSteam && (
                <button
                  type="button"
                  onClick={() => void handlePickExecutable()}
                  className="px-2.5 py-2 rounded-md bg-harbor-hover hover:bg-harbor-active text-harbor-textPrimary font-medium flex items-center gap-1 shrink-0 transition-colors"
                >
                  <FolderOpen className="w-3.5 h-3.5" strokeWidth={1.75} />
                  <span>Browse</span>
                </button>
              )}
            </div>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <div>
              <label
                htmlFor="shortcut-args"
                className="block text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted mb-1"
              >
                Arguments
              </label>
              <input
                id="shortcut-args"
                type="text"
                value={argumentsText}
                onChange={(e) => setArgumentsText(e.target.value)}
                placeholder="-novid"
                className="w-full px-2.5 py-1.5 rounded-md bg-harbor-rail border border-white/[0.06] text-[12.5px] text-harbor-textPrimary placeholder:text-harbor-textMuted focus:outline-none focus:border-harbor-blurple"
              />
            </div>
            <div>
              <label
                htmlFor="shortcut-desc"
                className="block text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted mb-1"
              >
                Description
              </label>
              <input
                id="shortcut-desc"
                type="text"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Subtitle"
                className="w-full px-2.5 py-1.5 rounded-md bg-harbor-rail border border-white/[0.06] text-[12.5px] text-harbor-textPrimary placeholder:text-harbor-textMuted focus:outline-none focus:border-harbor-blurple"
              />
            </div>
          </div>

          <div>
            <label
              htmlFor="shortcut-icon"
              className="block text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted mb-1"
            >
              Custom Icon (Optional)
            </label>
            <div className="flex items-center gap-1.5">
              <input
                id="shortcut-icon"
                type="text"
                value={customIconPath}
                onChange={(e) => setCustomIconPath(e.target.value)}
                placeholder="Default from target binary"
                className="flex-1 min-w-0 px-2.5 py-1.5 rounded-md bg-harbor-rail border border-white/[0.06] text-[12px] text-harbor-textPrimary placeholder:text-harbor-textMuted focus:outline-none focus:border-harbor-blurple"
              />
              {customIconPath && (
                <button
                  type="button"
                  aria-label="Clear custom icon"
                  onClick={() => setCustomIconPath("")}
                  className="p-1.5 rounded-md bg-harbor-rail hover:bg-harbor-hover text-harbor-textMuted hover:text-harbor-textPrimary"
                >
                  <X className="w-3.5 h-3.5" strokeWidth={1.75} />
                </button>
              )}
              <button
                type="button"
                onClick={() => void handlePickIcon()}
                className="px-2.5 py-1.5 rounded-md bg-harbor-hover hover:bg-harbor-active text-harbor-textPrimary font-medium shrink-0 transition-colors"
              >
                Browse
              </button>
            </div>
          </div>

          <div className="flex items-center justify-between pt-1">
            <label className="flex items-center gap-2 cursor-pointer text-harbor-textSecondary">
              <input
                type="checkbox"
                checked={runAsAdmin}
                onChange={(e) => setRunAsAdmin(e.target.checked)}
                className="rounded border-white/20 bg-harbor-rail text-harbor-blurple focus:ring-harbor-blurple"
              />
              <Shield
                className="w-3.5 h-3.5 text-harbor-warning"
                strokeWidth={1.75}
              />
              <span>Run as Administrator</span>
            </label>

            <label className="flex items-center gap-2 cursor-pointer text-harbor-textSecondary">
              <input
                type="checkbox"
                checked={pinned}
                onChange={(e) => setPinned(e.target.checked)}
                className="rounded border-white/20 bg-harbor-rail text-harbor-blurple focus:ring-harbor-blurple"
              />
              <span>Pin to top</span>
            </label>
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
              disabled={submitting}
              className="px-3.5 py-1.5 rounded-md bg-harbor-blurple hover:bg-harbor-blurpleHover text-white font-medium disabled:opacity-60 transition-colors"
            >
              {submitting
                ? "Saving..."
                : editingItem
                  ? "Save Shortcut"
                  : "Add Shortcut"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
