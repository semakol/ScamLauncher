import { useCallback, useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdaterState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "none" }
  | { kind: "available"; update: Update }
  | { kind: "downloading"; update: Update; done: number; total: number | null }
  | { kind: "error"; message: string };

/** Обновление лаунчера из GitHub Releases: автоматически при старте и по кнопке. */
export function useUpdater() {
  const [state, setState] = useState<UpdaterState>({ kind: "idle" });

  const checkNow = useCallback(async (silent: boolean) => {
    if (!silent) setState({ kind: "checking" });
    try {
      const update = await check();
      setState(update ? { kind: "available", update } : silent ? { kind: "idle" } : { kind: "none" });
    } catch (e) {
      setState(silent ? { kind: "idle" } : { kind: "error", message: String(e) });
    }
  }, []);

  useEffect(() => {
    if (!import.meta.env.DEV) checkNow(true);
  }, [checkNow]);

  const install = useCallback(async (update: Update) => {
    let done = 0;
    let total: number | null = null;
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") total = event.data.contentLength ?? null;
        if (event.event === "Progress") done += event.data.chunkLength;
        setState({ kind: "downloading", update, done, total });
      });
      await relaunch();
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  }, []);

  const dismiss = useCallback(() => setState({ kind: "idle" }), []);

  return { state, checkNow, install, dismiss };
}
