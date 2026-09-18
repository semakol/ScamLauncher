import { useEffect, useState } from "react";
import {
  deleteInstance,
  formatSize,
  getBuild,
  openInstanceDir,
  type BuildInfo,
  type Pack,
  type PackSettings,
} from "./api";
import RestoreDialog from "./RestoreDialog";

interface Props {
  pack: Pack;
  value: PackSettings;
  totalMemoryMb: number;
  /** Установлен ли билд (null — сборки на компьютере нет). */
  installedBuild: number | null;
  /** Идёт игра или подготовка — действия с файлами недоступны. */
  busy: boolean;
  onSave: (v: PackSettings) => void;
  onRepair: () => void;
  onRestore: (groups: string[]) => void;
  onDeleted: () => void;
  onClose: () => void;
}

const STEP = 512;
const gb = (mb: number) => `${(mb / 1024).toFixed(mb % 1024 === 0 ? 0 : 1)} ГБ`;

export default function PackSettingsView(props: Props) {
  const { pack, value, totalMemoryMb, installedBuild, busy } = props;
  const [build, setBuild] = useState<BuildInfo | null>(null);
  const [memory, setMemory] = useState<number | null>(value.memoryMb);
  const [jvm, setJvm] = useState(value.jvmArgs);
  const [server, setServer] = useState(value.server);
  const [showStatus, setShowStatus] = useState(value.serverStatus);
  const [autoConnect, setAutoConnect] = useState(value.autoConnect);
  const [optional, setOptional] = useState<Record<string, boolean>>(value.optional);
  const [backupWorlds, setBackupWorlds] = useState(value.backupWorlds);
  const [restoreOpen, setRestoreOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  useEffect(() => {
    getBuild(pack.id, pack.build).then(setBuild).catch(() => {});
  }, [pack.id, pack.build]);

  // Сохраняем сами, с небольшой задержкой — без отдельной кнопки.
  const { onSave } = props;
  useEffect(() => {
    const next = {
      memoryMb: memory,
      jvmArgs: jvm.trim(),
      server: server.trim(),
      serverStatus: showStatus,
      autoConnect,
      optional,
      backupWorlds,
    };
    const same =
      next.memoryMb === value.memoryMb &&
      next.jvmArgs === value.jvmArgs &&
      next.server === value.server &&
      next.serverStatus === value.serverStatus &&
      next.autoConnect === value.autoConnect &&
      next.backupWorlds === value.backupWorlds &&
      JSON.stringify(next.optional) === JSON.stringify(value.optional);
    if (same) return;
    const t = setTimeout(() => onSave(next), 400);
    return () => clearTimeout(t);
  }, [memory, jvm, server, showStatus, autoConnect, optional, backupWorlds, value, onSave]);
  const optionalGroups = build?.groups.filter((g) => g.optional && g.files > 0) ?? [];
  const address = server.trim() || pack.server;

  const recommended = build?.memoryRecommended ?? null;
  const fallback = recommended ?? 4096;
  const max = Math.max(2048, Math.floor((totalMemoryMb - 1024) / STEP) * STEP);
  const current = Math.min(memory ?? fallback, max);
  const tooMuch = current > totalMemoryMb * 0.75;
  const onceGroups = build?.groups.filter((g) => g.mode === "once" && g.files > 0) ?? [];

  async function remove() {
    setDeleteError(null);
    try {
      await deleteInstance(pack.id);
      setConfirmDelete(false);
      props.onDeleted();
    } catch (e) {
      setDeleteError(String(e));
    }
  }

  return (
    <div className="settings">
      <header className="settings-head">
        <h1>Настройки: {pack.name}</h1>
        <button className="ghost" onClick={props.onClose}>
          Готово
        </button>
      </header>

      <section className="card">
        <h2>Память и Java</h2>
        <label className="field">
          <span>
            Память: <strong>{gb(current)}</strong>
            {memory === null && " (как рекомендует сборка)"}
          </span>
          <input
            type="range"
            min={1024}
            max={max}
            step={STEP}
            value={current}
            onChange={(e) => setMemory(Number(e.target.value))}
          />
          <span className="muted small range-legend">
            <span>1 ГБ</span>
            <span>в компьютере {gb(totalMemoryMb)}</span>
            <span>{gb(max)}</span>
          </span>
        </label>
        {recommended !== null && (
          <p className="muted small">
            Сборка рекомендует {gb(recommended)}.{" "}
            {memory !== null && (
              <a
                href="#"
                onClick={(e) => {
                  e.preventDefault();
                  setMemory(null);
                }}
              >
                Вернуть
              </a>
            )}
          </p>
        )}
        {tooMuch && (
          <p className="warn small">
            Это больше 75% памяти компьютера — системе может не хватить, игра будет тормозить.
          </p>
        )}
        <label className="field spaced">
          <span>Дополнительные JVM-аргументы</span>
          <input
            className="text"
            value={jvm}
            placeholder="обычно не нужно"
            onChange={(e) => setJvm(e.target.value)}
          />
        </label>
      </section>

      {optionalGroups.length > 0 && (
        <section className="card">
          <h2>Дополнительные моды</h2>
          {optionalGroups.map((g) => {
            const on = optional[g.id] ?? g.enabledByDefault;
            return (
              <label key={g.id} className="optional-item">
                <input
                  type="checkbox"
                  checked={on}
                  onChange={(e) => setOptional((o) => ({ ...o, [g.id]: e.target.checked }))}
                />
                <span className="optional-text">
                  <span>{g.title}</span>
                  {g.description && <span className="muted small">{g.description}</span>}
                </span>
                <span className="muted small">{formatSize(g.size)}</span>
              </label>
            );
          })}
          <p className="muted small">
            Изменения применятся при следующем запуске. Выключенный мод можно заменить своей версией в
            «Клиентских модах».
          </p>
        </section>
      )}

      <section className="card">
        <h2>Сервер</h2>
        <label className="field">
          <span>Адрес сервера</span>
          <input
            className="text"
            value={server}
            placeholder={pack.server ?? "например mc.example.com"}
            onChange={(e) => setServer(e.target.value)}
          />
        </label>
        <p className="muted small">
          {pack.server
            ? `Пусто — сервер сборки ${pack.server}.`
            : "У сборки нет своего сервера — можно указать любой."}
        </p>
        <label className="toggle spaced-toggle">
          <input type="checkbox" checked={showStatus} onChange={(e) => setShowStatus(e.target.checked)} />
          Показывать статус сервера
        </label>
        <label className={`toggle${address ? "" : " disabled"}`}>
          <input
            type="checkbox"
            checked={autoConnect}
            disabled={!address}
            onChange={(e) => setAutoConnect(e.target.checked)}
          />
          Сразу заходить на сервер при запуске
        </label>
      </section>

      <section className="card">
        <h2>Файлы</h2>
        <div className="row">
          <button className="ghost" onClick={() => openInstanceDir(pack.id)}>
            Папка игры
          </button>
          <button
            className="ghost"
            onClick={() => openInstanceDir(pack.id, "clientMods")}
            title="Положи сюда свои моды (.jar) — они будут добавляться в игру и не пропадут при обновлениях"
          >
            Клиентские моды
          </button>
          {onceGroups.length > 0 && (
            <button className="ghost" disabled={busy || installedBuild === null} onClick={() => setRestoreOpen(true)}>
              Восстановить файлы…
            </button>
          )}
          <button
            className="ghost"
            disabled={busy || installedBuild === null}
            onClick={props.onRepair}
            title="Перепроверить все файлы игры и сборки и скачать испорченные"
          >
            Проверить и починить
          </button>
        </div>
        <p className="muted small">
          Свои моды клади в «Клиентские моды» — они добавятся в игру и не пропадут при обновлениях.
        </p>
        <label className="toggle spaced-toggle">
          <input type="checkbox" checked={backupWorlds} onChange={(e) => setBackupWorlds(e.target.checked)} />
          Бэкап миров перед обновлением сборки
        </label>
        <p className="muted small">
          Перед каждым обновлением миры упаковываются в архив, хранятся 3 последних.{" "}
          <a
            href="#"
            onClick={(e) => {
              e.preventDefault();
              openInstanceDir(pack.id, "worldBackups");
            }}
          >
            Открыть бэкапы
          </a>
        </p>
      </section>

      {build && (
        <section className="card">
          <h2>
            Состав · {build.files} файлов, {formatSize(build.totalSize)}
          </h2>
          <table className="groups">
            <tbody>
              {build.groups
                .filter((g) => g.files > 0)
                .map((g) => (
                  <tr key={g.id}>
                    <td>{g.title}</td>
                    <td className="muted">
                      {g.optional ? "по желанию" : g.mode === "sync" ? "всегда как в сборке" : "ставится один раз"}
                    </td>
                    <td className="num">{g.files}</td>
                    <td className="num">{formatSize(g.size)}</td>
                  </tr>
                ))}
            </tbody>
          </table>
        </section>
      )}

      <section className="card danger-zone">
        <h2>Удаление</h2>
        <div className="row">
          <button
            className="danger"
            disabled={busy || installedBuild === null}
            onClick={() => setConfirmDelete(true)}
          >
            Удалить сборку с компьютера
          </button>
        </div>
        <p className="muted small">
          {installedBuild === null
            ? "Сборка ещё не установлена."
            : "Папка сборки уйдёт в корзину. Сама сборка останется в списке — её можно установить заново."}
        </p>
      </section>

      {restoreOpen && (
        <RestoreDialog
          groups={onceGroups}
          onClose={() => setRestoreOpen(false)}
          onConfirm={(ids) => {
            setRestoreOpen(false);
            props.onRestore(ids);
          }}
        />
      )}
      {confirmDelete && (
        <div className="modal-backdrop" onClick={() => setConfirmDelete(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h2>Удалить «{pack.name}» с компьютера?</h2>
            <p>
              Удалится вся папка сборки: <strong>миры</strong>, настройки, скриншоты, клиентские моды.
              Она попадёт в корзину — оттуда её можно вернуть.
            </p>
            <p className="muted small">Хочешь сохранить миры — сначала скопируй папку saves из «Папка игры».</p>
            {deleteError && <div className="load-error">{deleteError}</div>}
            <div className="modal-actions">
              <button className="ghost" onClick={() => setConfirmDelete(false)}>
                Отмена
              </button>
              <button className="danger" onClick={remove}>
                Удалить
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
