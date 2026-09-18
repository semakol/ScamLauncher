import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

type State =
  | { kind: "idle" }
  | { kind: "available"; update: Update }
  | { kind: "downloading"; update: Update; done: number; total: number | null }
  | { kind: "error"; message: string };

/** Проверяет GitHub Releases и предлагает игроку обновиться. */
export default function UpdateBanner() {
  const [state, setState] = useState<State>({ kind: "idle" });
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    if (import.meta.env.DEV) return;
    check()
      .then((update) => update && setState({ kind: "available", update }))
      .catch((e) => console.warn("Проверка обновлений не удалась:", e));
  }, []);

  if (dismissed || state.kind === "idle") return null;

  async function install(update: Update) {
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
  }

  if (state.kind === "error") {
    return (
      <div className="banner banner-error">
        <span>Не удалось обновить лаунчер: {state.message}</span>
        <button className="ghost" onClick={() => setDismissed(true)}>
          Закрыть
        </button>
      </div>
    );
  }

  const { update } = state;
  if (state.kind === "downloading") {
    const pct = state.total ? Math.round((state.done / state.total) * 100) : null;
    return (
      <div className="banner">
        <span>
          Загрузка версии {update.version}… {pct !== null ? `${pct}%` : `${(state.done / 1e6).toFixed(1)} МБ`}
        </span>
      </div>
    );
  }

  return (
    <div className="banner">
      <div className="banner-text">
        <strong>Доступна версия {update.version}</strong>
        {update.body && <pre className="notes">{update.body}</pre>}
      </div>
      <div className="banner-actions">
        <button className="ghost" onClick={() => setDismissed(true)}>
          Позже
        </button>
        <button onClick={() => install(update)}>Обновить</button>
      </div>
    </div>
  );
}
