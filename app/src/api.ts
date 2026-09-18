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
  /** Автор удалил сборку с сервера; у игрока она ещё установлена. */
  removed: boolean;
  icon: string | null;
  background: string | null;
  server: string | null;
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
  /** Сервер недоступен — показана сохранённая копия. */
  offline: boolean;
}

export interface GroupInfo {
  id: string;
  title: string;
  mode: GroupMode;
  files: number;
  size: number;
  optional: boolean;
  enabledByDefault: boolean;
  description: string | null;
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

// ---------- игра ----------

export interface LogLine {
  level: string | null;
  thread: string | null;
  text: string;
  stderr: boolean;
}

export interface SyncReport {
  downloaded: number;
  downloadedBytes: number;
  removed: string[];
  installedGroups: { id: string; title: string; reason: "first" | "revision" | "missing" | "requested" }[];
  clientCopied: string[];
  clientRemoved: string[];
  conflicts: { file: string; modIds: string[] }[];
  backup: string | null;
  worldsBackup: string | null;
}

export type DoneTask = "install" | "repair" | "restore";

export type GameEvent =
  | { kind: "stage"; pack: string; text: string }
  | { kind: "bytes"; pack: string; done: number; total: number }
  | { kind: "synced"; pack: string; report: SyncReport }
  | { kind: "done"; pack: string; task: DoneTask; report: SyncReport }
  | { kind: "started"; pack: string }
  | { kind: "log"; pack: string; lines: LogLine[] }
  | {
      kind: "exited";
      pack: string;
      code: number | null;
      crashed: boolean;
      killed: boolean;
      crashReport: string | null;
    }
  | { kind: "failed"; pack: string; message: string };

export const play = (pack: string, build: number, nick: string) =>
  invoke<void>("play", { pack, build, nick });

export const repair = (pack: string, build: number) => invoke<void>("repair", { pack, build });

export const install = (pack: string, build: number) => invoke<void>("install", { pack, build });

export const instanceInfo = (pack: string) =>
  invoke<{ installedBuild: number | null }>("instance_info", { pack });

export const deleteInstance = (pack: string) => invoke<void>("delete_instance", { pack });

export const openLink = (url: string) => invoke<void>("open_link", { url });

export const restore = (pack: string, build: number, groups: string[]) =>
  invoke<void>("restore", { pack, build, groups });

export const killGame = () => invoke<boolean>("kill_game");

export const runningPack = () => invoke<string | null>("running_pack");

export const openInstanceDir = (pack: string, what?: "clientMods" | "worldBackups") =>
  invoke<void>("open_instance_dir", { pack, what: what ?? null });

export const openGameFile = (path: string) => invoke<void>("open_game_file", { path });

export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    document.body.appendChild(area);
    area.select();
    document.execCommand("copy");
    area.remove();
  }
}

export function formatLog(lines: LogLine[]): string {
  return lines.map((l) => (l.level ? `[${l.level}] ${l.text}` : l.text)).join("\n");
}

// ---------- настройки ----------

export interface PackSettings {
  memoryMb: number | null;
  jvmArgs: string;
  /** Свой адрес сервера; пусто — из сборки. */
  server: string;
  serverStatus: boolean;
  autoConnect: boolean;
  /** Выбор по опциональным модам: id группы → включена. */
  optional: Record<string, boolean>;
  /** Бэкап миров перед обновлением сборки. */
  backupWorlds: boolean;
}

export const DEFAULT_PACK_SETTINGS: PackSettings = {
  memoryMb: null,
  jvmArgs: "",
  server: "",
  serverStatus: true,
  autoConnect: false,
  optional: {},
  backupWorlds: true,
};

/** Настройки сборки с умолчаниями (старые сохранения могут быть без новых полей). */
export function packSettingsOf(settings: Settings | undefined, pack: string): PackSettings {
  return { ...DEFAULT_PACK_SETTINGS, ...(settings?.packs[pack] ?? {}) };
}

export interface ServerStatus {
  online: number;
  max: number;
  version: string;
  motd: string;
  latencyMs: number;
}

export const serverStatus = (address: string) => invoke<ServerStatus>("server_status", { address });

export interface Settings {
  gameDir: string | null;
  beta: boolean;
  nick: string;
  jvmArgs: string;
  packs: Record<string, PackSettings>;
  /** id новостей, которые игрок уже видел. */
  seenNews: string[];
}

export interface SettingsInfo {
  settings: Settings;
  gameDir: string;
  defaultGameDir: string;
  totalMemoryMb: number;
}

export const getSettings = () => invoke<SettingsInfo>("get_settings");

export const saveSettings = (settings: Settings) => invoke<void>("save_settings", { settings });

export const moveGameDir = (path: string, moveFiles: boolean) =>
  invoke<void>("move_game_dir", { path, moveFiles });

const images = new Map<string, Promise<string>>();

/** Картинка сборки по sha1 как data URL (с кэшем на время сеанса). */
export function getImage(sha1: string): Promise<string> {
  let p = images.get(sha1);
  if (!p) {
    p = invoke<string>("get_image", { sha1 });
    p.catch(() => images.delete(sha1));
    images.set(sha1, p);
  }
  return p;
}
