import { useCallback, useEffect, useState } from "react";

export const DEFAULT_TABLE_PAGE_SIZE = 20;
export const TABLE_PAGE_SIZE_STORAGE_KEY = "holdings_table_page_size";

interface PageSizeStorage {
  getItem: (key: string) => string | null;
  setItem: (key: string, value: string) => void;
}

function isValidPageSize(pageSize: number): boolean {
  return Number.isInteger(pageSize) && pageSize > 0;
}

export function loadTablePageSize(storage: PageSizeStorage): number {
  const pageSize = Number(storage.getItem(TABLE_PAGE_SIZE_STORAGE_KEY));
  return isValidPageSize(pageSize) ? pageSize : DEFAULT_TABLE_PAGE_SIZE;
}

export function saveTablePageSize(storage: PageSizeStorage, pageSize: number): void {
  if (isValidPageSize(pageSize)) {
    storage.setItem(TABLE_PAGE_SIZE_STORAGE_KEY, String(pageSize));
  }
}

const listeners = new Set<(pageSize: number) => void>();

export function useTablePageSize() {
  const [pageSize, setPageSize] = useState(() => loadTablePageSize(localStorage));

  useEffect(() => {
    listeners.add(setPageSize);
    return () => {
      listeners.delete(setPageSize);
    };
  }, []);

  const onShowSizeChange = useCallback((_current: number, nextPageSize: number) => {
    saveTablePageSize(localStorage, nextPageSize);
    listeners.forEach((listener) => listener(nextPageSize));
  }, []);

  return { pageSize, onShowSizeChange };
}
