import { useCallback, useEffect, useRef, useState } from "react";
import { ipc } from "../lib/ipc";
import type { DockItem } from "../types/dock";

const MAX_CONCURRENT_ICON_FETCHES = 6;

interface IconEntry {
  signature: string;
  url: string | null;
  status: "loading" | "ready";
}

interface QueuedRequest {
  id: string;
  signature: string;
}

export function getIconSignature(item: DockItem): string {
  return `${item.custom_icon_path ?? ""}|${item.icon_source_path ?? ""}|${item.path}`;
}

export function useIconStore(items: DockItem[]) {
  const [entries, setEntries] = useState<Record<string, IconEntry>>({});
  const entriesRef = useRef<Record<string, IconEntry>>({});
  const queueRef = useRef<QueuedRequest[]>([]);
  const queuedIdsRef = useRef<Set<string>>(new Set());
  const inFlightRef = useRef<number>(0);
  const mountedRef = useRef<boolean>(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Invalidate cached icons immediately whenever an item's icon source fields change or disappear.
  useEffect(() => {
    const activeMap = new Map<string, string>();
    for (const item of items) {
      activeMap.set(item.id, getIconSignature(item));
    }

    let changed = false;
    const next: Record<string, IconEntry> = {};
    for (const [id, entry] of Object.entries(entriesRef.current)) {
      const sig = activeMap.get(id);
      if (sig !== undefined && sig === entry.signature) {
        next[id] = entry;
      } else {
        changed = true;
      }
    }
    if (changed) {
      entriesRef.current = next;
      setEntries(next);
    }
  }, [items]);

  const pumpQueue = useCallback(() => {
    while (
      inFlightRef.current < MAX_CONCURRENT_ICON_FETCHES &&
      queueRef.current.length > 0
    ) {
      const nextReq = queueRef.current.shift();
      if (!nextReq) break;
      queuedIdsRef.current.delete(nextReq.id);

      const currentEntry = entriesRef.current[nextReq.id];
      if (
        currentEntry &&
        currentEntry.signature === nextReq.signature &&
        currentEntry.status === "ready"
      ) {
        continue;
      }

      inFlightRef.current += 1;

      const loadingEntry: IconEntry = {
        signature: nextReq.signature,
        url: null,
        status: "loading",
      };
      entriesRef.current = {
        ...entriesRef.current,
        [nextReq.id]: loadingEntry,
      };

      void ipc
        .getItemIcon(nextReq.id)
        .then((filePath) => {
          if (!mountedRef.current) return;
          const latest = entriesRef.current[nextReq.id];
          // Drop stale result if the item's signature changed while fetching.
          if (latest && latest.signature !== nextReq.signature) {
            return;
          }
          const resolvedUrl = ipc.toAssetUrl(filePath);
          const readyEntry: IconEntry = {
            signature: nextReq.signature,
            url: resolvedUrl,
            status: "ready",
          };
          entriesRef.current = {
            ...entriesRef.current,
            [nextReq.id]: readyEntry,
          };
          setEntries(entriesRef.current);
        })
        .catch(() => {
          if (!mountedRef.current) return;
          const fallbackEntry: IconEntry = {
            signature: nextReq.signature,
            url: null,
            status: "ready",
          };
          entriesRef.current = {
            ...entriesRef.current,
            [nextReq.id]: fallbackEntry,
          };
          setEntries(entriesRef.current);
        })
        .finally(() => {
          inFlightRef.current = Math.max(0, inFlightRef.current - 1);
          if (mountedRef.current) {
            pumpQueue();
          }
        });
    }
  }, []);

  const requestIcon = useCallback(
    (item: DockItem) => {
      const sig = getIconSignature(item);
      const existing = entriesRef.current[item.id];
      if (existing && existing.signature === sig) {
        return;
      }
      if (queuedIdsRef.current.has(item.id)) {
        // Update queued signature if item changed before dequeue.
        queueRef.current = queueRef.current.map((q) =>
          q.id === item.id ? { id: item.id, signature: sig } : q,
        );
        return;
      }
      queuedIdsRef.current.add(item.id);
      queueRef.current.push({ id: item.id, signature: sig });
      pumpQueue();
    },
    [pumpQueue],
  );

  const getIconUrl = useCallback(
    (item: DockItem): string | null => {
      const entry = entries[item.id];
      if (!entry) return null;
      if (entry.signature !== getIconSignature(item)) return null;
      return entry.url;
    },
    [entries],
  );

  const invalidateIcon = useCallback((id: string) => {
    if (!(id in entriesRef.current)) return;
    const copy = { ...entriesRef.current };
    delete copy[id];
    entriesRef.current = copy;
    setEntries(copy);
  }, []);

  return { getIconUrl, requestIcon, invalidateIcon };
}
