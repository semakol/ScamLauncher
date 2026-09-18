import type { useUpdater } from "./useUpdater";

type Updater = ReturnType<typeof useUpdater>;

/** Плашка «доступна новая версия лаунчера». */
export default function UpdateBanner({ updater }: { updater: Updater }) {
  const { state } = updater;
  if (state.kind === "error") {
    return (
      <div className="banner banner-error">
        <span>Не удалось обновить лаунчер: {state.message}</span>
        <button className="ghost" onClick={updater.dismiss}>
          Закрыть
        </button>
      </div>
    );
  }
  if (state.kind === "downloading") {
    const pct = state.total ? Math.round((state.done / state.total) * 100) : null;
    return (
      <div className="banner">
        <span>
          Загрузка версии {state.update.version}…{" "}
          {pct !== null ? `${pct}%` : `${(state.done / 1e6).toFixed(1)} МБ`}
        </span>
      </div>
    );
  }
  if (state.kind !== "available") return null;
  const { update } = state;
  return (
    <div className="banner">
      <div className="banner-text">
        <strong>Доступна версия {update.version}</strong>
        {update.body && <pre className="notes">{update.body}</pre>}
      </div>
      <div className="banner-actions">
        <button className="ghost" onClick={updater.dismiss}>
          Позже
        </button>
        <button onClick={() => updater.install(update)}>Обновить</button>
      </div>
    </div>
  );
}
