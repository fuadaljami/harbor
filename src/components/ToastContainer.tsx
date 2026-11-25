import { AlertTriangle, CheckCircle2, Info, X, XCircle } from "lucide-react";

export type ToastVariant = "info" | "success" | "warning" | "error";

export interface ToastMessage {
  id: string;
  variant: ToastVariant;
  title: string;
  detail?: string;
}

interface ToastContainerProps {
  toasts: ToastMessage[];
  onDismiss: (id: string) => void;
}

const VARIANT_STYLES: Record<
  ToastVariant,
  { border: string; iconClass: string; Icon: typeof Info }
> = {
  info: {
    border: "border-l-harbor-blurple",
    iconClass: "text-harbor-blurple",
    Icon: Info,
  },
  success: {
    border: "border-l-harbor-success",
    iconClass: "text-harbor-success",
    Icon: CheckCircle2,
  },
  warning: {
    border: "border-l-harbor-warning",
    iconClass: "text-harbor-warning",
    Icon: AlertTriangle,
  },
  error: {
    border: "border-l-harbor-danger",
    iconClass: "text-harbor-danger",
    Icon: XCircle,
  },
};

export function ToastContainer({ toasts, onDismiss }: ToastContainerProps) {
  if (toasts.length === 0) {
    return null;
  }

  return (
    <div className="pointer-events-none absolute inset-x-2 bottom-[58px] z-40 flex flex-col gap-1.5">
      {toasts.map((toast) => {
        const { border, iconClass, Icon } = VARIANT_STYLES[toast.variant];
        return (
          <div
            key={toast.id}
            role="status"
            className={`pointer-events-auto flex items-start gap-2 rounded-md bg-harbor-floating border border-white/[0.08] border-l-[3px] ${border} px-2.5 py-2 shadow-xl transition-all duration-150`}
          >
            <Icon
              className={`w-4 h-4 mt-0.5 shrink-0 ${iconClass}`}
              strokeWidth={1.75}
            />
            <div className="min-w-0 flex-1">
              <div className="text-xs font-medium text-harbor-textPrimary leading-tight truncate">
                {toast.title}
              </div>
              {toast.detail && (
                <div className="mt-0.5 text-[11px] text-harbor-textMuted leading-snug line-clamp-2">
                  {toast.detail}
                </div>
              )}
            </div>
            <button
              type="button"
              onClick={() => onDismiss(toast.id)}
              className="p-0.5 rounded text-harbor-textMuted hover:text-harbor-textPrimary hover:bg-harbor-hover shrink-0"
              aria-label="Dismiss notification"
            >
              <X className="w-3.5 h-3.5" strokeWidth={1.75} />
            </button>
          </div>
        );
      })}
    </div>
  );
}
