import { copyText, formatLog, openGameFile, type LogLine } from "./api";
import type { GameState } from "./useGame";

interface Props {
  state: GameState;
  logs: LogLine[];
  onShowConsole: () => void;
  onDismiss: () => void;
}

/** Сообщение о сбое игры или ошибке запуска. */
export default function GameNotice({ state, logs, onShowConsole, onDismiss }: Props) {
  if (state.kind === "failed") {
    return (
      <div className="banner banner-error">
        <div className="banner-text">
          <strong>Не удалось запустить игру</strong>
          <div className="notes">{state.message}</div>
        </div>
        <div className="banner-actions">
          <button className="ghost" onClick={onDismiss}>
            Закрыть
          </button>
        </div>
      </div>
    );
  }
  if (state.kind === "exited" && state.crashed) {
    const report = state.crashReport;
    return (
      <div className="banner banner-error">
        <div className="banner-text">
          <strong>Игра завершилась с ошибкой{state.code !== null ? ` (код ${state.code})` : ""}</strong>
          <div className="notes">
            {logs
              .filter((l) => l.level === "ERROR" || l.level === "FATAL")
              .slice(-3)
              .map((l) => l.text.split("\n")[0])
              .join("\n") || "Подробности — в консоли."}
          </div>
        </div>
        <div className="banner-actions wrap">
          <button className="ghost" onClick={onShowConsole}>
            Консоль
          </button>
          {report && (
            <button className="ghost" onClick={() => openGameFile(report)}>
              Отчёт о сбое
            </button>
          )}
          <button className="ghost" onClick={() => copyText(formatLog(logs))}>
            Скопировать лог
          </button>
          <button className="ghost" onClick={onDismiss}>
            Закрыть
          </button>
        </div>
      </div>
    );
  }
  return null;
}
