import { useEffect, useRef, type RefObject } from "react";
import type { DockItem } from "../types/dock";
import {
  categoryForShortcutDigit,
  cycleCategory,
  resolveHighlightAfterListChange,
  stepHighlightedId,
  type RailCategory,
} from "../lib/keyboard";

interface UseKeyboardNavOptions {
  expanded: boolean;
  isClosing: boolean;
  itemsById: ReadonlyMap<string, DockItem>;
  visibleIds: readonly string[];
  highlightedId: string | null;
  selectedCategory: RailCategory;
  searchQuery: string;
  modalOpen: boolean;
  contextMenuOpen: boolean;
  hotkeyRecorderActive: boolean;
  searchInputRef: RefObject<HTMLInputElement>;
  cardRefs: RefObject<Map<string, HTMLDivElement>>;
  onHighlightChange: (id: string | null) => void;
  onSelectCategory: (category: RailCategory) => void;
  onClearSearch: () => void;
  onCollapseDock: () => void;
  onCloseContextMenu: () => void;
  onCloseModal: () => void;
  onCancelHotkeyRecorder?: () => void;
  onLaunchItem: (item: DockItem, runAsAdmin: boolean) => void;
}

const ENTER_LAUNCH_DEBOUNCE_MS = 500;

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  const tag = target.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    target.isContentEditable
  );
}

function isFocusedInteractiveNonCard(
  activeEl: Element | null,
  searchInput: HTMLInputElement | null,
): string | null {
  if (!(activeEl instanceof HTMLElement)) {
    return null;
  }
  if (searchInput && activeEl === searchInput) {
    return null;
  }
  const cardId = activeEl.getAttribute("data-shortcut-id");
  if (cardId) {
    return cardId;
  }
  return null;
}

function shouldDeferEnterToFocusedControl(
  activeEl: Element | null,
  searchInput: HTMLInputElement | null,
): boolean {
  if (!(activeEl instanceof HTMLElement)) {
    return false;
  }
  if (searchInput && activeEl === searchInput) {
    return false;
  }
  if (activeEl.hasAttribute("data-shortcut-id")) {
    return false;
  }
  const tag = activeEl.tagName;
  if (tag === "BUTTON" || tag === "A" || tag === "SELECT" || tag === "TEXTAREA") {
    return true;
  }
  const role = activeEl.getAttribute("role");
  return role === "button" || role === "menuitem" || role === "switch";
}

export function useKeyboardNav({
  expanded,
  isClosing,
  itemsById,
  visibleIds,
  highlightedId,
  selectedCategory,
  searchQuery,
  modalOpen,
  contextMenuOpen,
  hotkeyRecorderActive,
  searchInputRef,
  cardRefs,
  onHighlightChange,
  onSelectCategory,
  onClearSearch,
  onCollapseDock,
  onCloseContextMenu,
  onCloseModal,
  onCancelHotkeyRecorder,
  onLaunchItem,
}: UseKeyboardNavOptions) {
  const prevExpandedRef = useRef<boolean>(false);
  const lastEnterLaunchRef = useRef<number>(0);

  // Auto-focus SearchBar cleanly on expand transition (false -> true).
  useEffect(() => {
    const wasExpanded = prevExpandedRef.current;
    prevExpandedRef.current = expanded;

    if (!wasExpanded && expanded && !isClosing) {
      const focusSearch = () => {
        if (modalOpen || contextMenuOpen) return;
        const input = searchInputRef.current;
        if (!input) return;
        input.focus();
        if (input.value.length > 0) {
          input.select();
        }
      };

      const rafId = requestAnimationFrame(focusSearch);
      const timerId = window.setTimeout(focusSearch, 60);
      return () => {
        cancelAnimationFrame(rafId);
        window.clearTimeout(timerId);
      };
    }
    return undefined;
  }, [expanded, isClosing, modalOpen, contextMenuOpen, searchInputRef]);

  // Keep highlighted item consistent when visible items or search query change.
  useEffect(() => {
    const isSearching = searchQuery.trim().length > 0;
    const nextId = resolveHighlightAfterListChange(
      visibleIds,
      highlightedId,
      isSearching,
    );
    if (nextId !== highlightedId) {
      onHighlightChange(nextId);
    }
  }, [visibleIds, highlightedId, searchQuery, onHighlightChange]);

  useEffect(() => {
    if (!expanded || isClosing) {
      return undefined;
    }

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.isComposing) {
        return;
      }

      if (e.key === "Escape") {
        e.preventDefault();
        if (hotkeyRecorderActive) {
          onCancelHotkeyRecorder?.();
          return;
        }
        if (contextMenuOpen) {
          onCloseContextMenu();
          return;
        }
        if (modalOpen) {
          onCloseModal();
          return;
        }
        if (searchQuery.length > 0) {
          onClearSearch();
          searchInputRef.current?.focus();
          return;
        }
        onCollapseDock();
        return;
      }

      if (hotkeyRecorderActive || contextMenuOpen || modalOpen) {
        return;
      }

      const lower = e.key.toLowerCase();

      if ((e.ctrlKey || e.metaKey) && !e.altKey && lower === "k") {
        e.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
        return;
      }

      if (
        e.key === "/" &&
        !e.ctrlKey &&
        !e.metaKey &&
        !e.altKey &&
        !isEditableTarget(e.target)
      ) {
        e.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
        return;
      }

      if (e.ctrlKey && !e.altKey && !e.metaKey) {
        const cat = categoryForShortcutDigit(e.key);
        if (cat !== null) {
          e.preventDefault();
          onSelectCategory(cat);
          return;
        }
      }

      if (e.key === "Tab" && !e.ctrlKey && !e.altKey && !e.metaKey) {
        e.preventDefault();
        onSelectCategory(
          cycleCategory(selectedCategory, e.shiftKey ? "prev" : "next"),
        );
        return;
      }

      if (
        (e.key === "ArrowDown" || e.key === "ArrowUp") &&
        !e.altKey &&
        !e.metaKey
      ) {
        e.preventDefault();
        const nextId = stepHighlightedId(
          visibleIds,
          highlightedId,
          e.key === "ArrowDown" ? "next" : "prev",
        );
        onHighlightChange(nextId);
        if (nextId !== null) {
          cardRefs.current?.get(nextId)?.scrollIntoView({ block: "nearest" });
        }
        return;
      }

      if (e.key === "Enter" && !e.altKey && !e.metaKey && !e.shiftKey) {
        if (e.repeat) {
          return;
        }
        const activeEl = document.activeElement;
        const searchInput = searchInputRef.current;
        if (shouldDeferEnterToFocusedControl(activeEl, searchInput)) {
          return;
        }

        const focusedCardId = isFocusedInteractiveNonCard(activeEl, searchInput);
        const isSearching = searchQuery.trim().length > 0;
        const targetId =
          focusedCardId ??
          highlightedId ??
          (isSearching ? (visibleIds[0] ?? null) : null);

        if (!targetId) {
          return;
        }
        const targetItem = itemsById.get(targetId);
        if (!targetItem) {
          return;
        }

        e.preventDefault();
        const now = Date.now();
        if (now - lastEnterLaunchRef.current < ENTER_LAUNCH_DEBOUNCE_MS) {
          return;
        }
        lastEnterLaunchRef.current = now;
        onLaunchItem(targetItem, e.ctrlKey);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [
    expanded,
    isClosing,
    itemsById,
    visibleIds,
    highlightedId,
    selectedCategory,
    searchQuery,
    modalOpen,
    contextMenuOpen,
    hotkeyRecorderActive,
    searchInputRef,
    cardRefs,
    onHighlightChange,
    onSelectCategory,
    onClearSearch,
    onCollapseDock,
    onCloseContextMenu,
    onCloseModal,
    onCancelHotkeyRecorder,
    onLaunchItem,
  ]);
}
