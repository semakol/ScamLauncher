import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getVersion } from "@tauri-apps/api/app";
import { moveGameDir, type Settings, type SettingsInfo } from "./api";
import type { useUpdater } from "./useUpdater";

interface Props {
  info: SettingsInfo;
  error: string | null;
  busy: boolean;
  update: (patch: (s: Settings) => Settings) => Promise<void>;
  reload: () => Promise<void>;
  updater: ReturnType<typeof useUpdater>;
  onClose: () => void;
}

export default function SettingsView({ info, error, busy, update, reload, updater, onClose }: Props) {
  const s = info.settings;
  const [jvm, setJvm] = useState(s.jvmArgs);
  const [moveTo, setMoveTo] = useState<string | null>(null);
  const [moving, setMoving] = useState(false);
  const [moveError, setMoveError] = useState<string | null>(null);
  const [version, setVersion] = useState("");
  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);

  async function pickFolder() {
    const dir = await open({ directory: true, title: "Новая папка для игры" });
    if (typeof dir === "string") {
      setMoveError(null);
      setMoveTo(dir);
    }
  }

  async function applyMove(path: string, moveFiles: boolean) {
    setMoving(true);
    setMoveError(null);
    try {
      await moveGameDir(path, moveFiles);
      await reload();
      setMoveTo(null);
    } catch (e) {
      setMoveError(String(e));
    } finally {
      setMoving(false);
    }
  }

  const custom = info.gameDir !== info.defaultGameDir;
  const u = updater.state;

  return (
    <div className="settings">
      <header className="settings-head">
        <h1>Настройки</h1>
        <button className="ghost" onClick={onClose}>
          Готово
        </button>
      </header>
      {error && <div className="load-error">{error}</div>}

      <section className="card">
        <h2>Сборки</h2>
        <label className="toggle">
          <input
            type="checkbox"
            checked={s.beta}
            onChange={(e) => update((x) => ({ ...x, beta: e.target.checked }))}
          />
          Бета-версии сборок
        </label>
        <p className="muted small">
          Если у сборки есть бета новее релиза, будет запускаться она. Папка игры та же — моды
          переключатся сами, настройки останутся твоими.
        </p>
      </section>

      <section className="card">
        <h2>Папка игры</h2>
        <div className="path">{info.gameDir}</div>
        <p className="muted small">Здесь лежат Java, файлы Minecraft и папки всех сборок с мирами.</p>
        <div className="row">
          <button className="ghost" disabled={busy || moving} onClick={pickFolder}>
            Изменить…
          </button>
          {custom && (
            <button className="ghost" disabled={busy || moving} onClick={() => setMoveTo(info.defaultGameDir)}>
              Вернуть стандартную
            </button>
          )}
        </div>
        {busy && <p className="muted small">Сначала закрой игру.</p>}
        {moveTo && (
          <div className="move-box">
            <div>
              Новая папка: <span className="path inline">{moveTo}</span>
            </div>
            <div className="row">
              <button disabled={moving} onClick={() => applyMove(moveTo, true)}>
                {moving ? "Переносим файлы…" : "Перенести файлы"}
              </button>
              <button className="ghost" disabled={moving} onClick={() => applyMove(moveTo, false)}>
                Не переносить
              </button>
              <button className="ghost" disabled={moving} onClick={() => setMoveTo(null)}>
                Отмена
              </button>
            </div>
            <p className="muted small">
              «Не переносить» — всё скачается заново, а старая папка останется как есть (миры в ней
              тоже).
            </p>
            {moveError && <div className="load-error">{moveError}</div>}
          </div>
        )}
      </section>

      <section className="card">
        <h2>Java</h2>
        <label className="field">
          <span>Дополнительные JVM-аргументы для всех сборок</span>
          <input
            className="text"
            value={jvm}
            placeholder="например -XX:+UseZGC"
            onChange={(e) => setJvm(e.target.value)}
            onBlur={() => jvm !== s.jvmArgs && update((x) => ({ ...x, jvmArgs: jvm }))}
          />
        </label>
        <p className="muted small">Память задаётся в настройках каждой сборки.</p>
      </section>

      <section className="card">
        <h2>О лаунчере</h2>
        <div className="row">
          <span>ScamLauncher {version}</span>
          <button
            className="ghost"
            disabled={u.kind === "checking" || u.kind === "downloading"}
            onClick={() => updater.checkNow(false)}
          >
            Проверить обновления
          </button>
        </div>
        <p className="muted small">
          {u.kind === "checking" && "Проверяю…"}
          {u.kind === "none" && "Установлена последняя версия."}
          {u.kind === "available" && `Доступна версия ${u.update.version} — нажми «Обновить» вверху.`}
          {u.kind === "error" && `Не удалось проверить: ${u.message}`}
        </p>
      </section>
    </div>
  );
}
