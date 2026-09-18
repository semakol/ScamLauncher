import { invoke } from "@tauri-apps/api/core";

export type Channel = "stable" | "beta";
export type GroupMode = "sync" | "once";

export interface Pack {
  id: string;
  name: string;
  description: string | null;
  minecraft: string;
  loader: string;
  channel: Channel;
  build: number;
  version: string;
  hasBeta: boolean;
  updated: string;
}

export interface NewsItem {
  id: string;
  date: string;
  title: string;
  text: string;
  pack: string | null;
}

export interface Catalog {
  packs: Pack[];
  news: NewsItem[];
}

export interface GroupInfo {
  id: string;
  title: string;
  mode: GroupMode;
  files: number;
  size: number;
}

export interface BuildInfo {
  pack: string;
  build: number;
  version: string;
  created: string;
  minecraft: string;
  loader: string;
  loaderVersion: string | null;
  changelog: string | null;
  memoryRecommended: number | null;
  files: number;
  totalSize: number;
  groups: GroupInfo[];
}

export const getCatalog = (beta: boolean) => invoke<Catalog>("get_catalog", { beta });

export const getBuild = (pack: string, build: number) =>
  invoke<BuildInfo>("get_build", { pack, build });

export function formatSize(bytes: number): string {
  const units = ["Б", "КБ", "МБ", "ГБ"];
  let v = bytes;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return u === 0 ? `${bytes} Б` : `${v.toFixed(1)} ${units[u]}`;
}

export function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString("ru-RU", {
    day: "numeric",
    month: "long",
    year: "numeric",
  });
}

/** localStorage может быть недоступен — тогда просто не запоминаем. */
export const storage = {
  get(key: string): string | null {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  },
  set(key: string, value: string) {
    try {
      localStorage.setItem(key, value);
    } catch {
      /* не критично */
    }
  },
};
