import type { RefObject } from "react";
import { Search, X } from "lucide-react";

interface SearchBarProps {
  query: string;
  inputRef: RefObject<HTMLInputElement>;
  onChange: (value: string) => void;
}

export function SearchBar({ query, inputRef, onChange }: SearchBarProps) {
  return (
    <div className="px-3 pt-2.5 pb-1.5 shrink-0">
      <div className="h-[34px] px-2.5 rounded-[6px] bg-harbor-rail border border-white/[0.05] focus-within:border-harbor-blurple/80 flex items-center gap-2 transition-colors">
        <Search
          className="w-3.5 h-3.5 text-harbor-textMuted shrink-0"
          strokeWidth={1.75}
        />
        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(e) => onChange(e.target.value)}
          placeholder="Find or launch a program..."
          aria-label="Find or launch a program"
          className="flex-1 min-w-0 bg-transparent text-[13px] text-harbor-textPrimary placeholder:text-harbor-textMuted focus:outline-none"
        />
        {query.length > 0 ? (
          <button
            type="button"
            aria-label="Clear search"
            onClick={() => {
              onChange("");
              inputRef.current?.focus();
            }}
            className="p-0.5 rounded text-harbor-textMuted hover:text-harbor-textPrimary hover:bg-harbor-hover transition-colors"
          >
            <X className="w-3.5 h-3.5" strokeWidth={1.75} />
          </button>
        ) : (
          <span className="px-1.5 py-0.5 rounded bg-harbor-panel border border-white/[0.06] text-[10px] font-medium text-harbor-textMuted shrink-0">
            Ctrl+K
          </span>
        )}
      </div>
    </div>
  );
}
