import { useEffect, useState } from "react";
import {
  formatDate,
  formatSize,
  getBuild,
  openInstanceDir,
  type BuildInfo,
  type NewsItem,
  type Pack,
} from "./api";
import NewsList from "./NewsList";
import RestoreDialog from "./RestoreDialog";
import Markdown from "./Markdown";
import PackSettingsDialog from "./PackSettingsDialog";
import { useImage } from "./PackImage";
import type { PackSettings } from "./api";

type Load =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; build: BuildInfo };

interface Props {
  pack: Pack;
  news: NewsItem[];
  /** Идёт запуск, починка или игра — действия с файлами недоступны. */
  busy: boolean;
  onRepair: () => void;
  onRestore: (groups: string[]) => void;
  packSettings: PackSettings;
  totalMemoryMb: number;
  onSaveSettings: (v: PackSettings) => void;
}

export default function PackView({
  pack,
  news,
  busy,
  onRepair,
  onRestore,
  packSettings,
  totalMemoryMb,
  onSaveSettings,
}: Props) {
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [restoreOpen, setRestoreOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const background = useImage(pack.background);

  useEffect(() => {
    let alive = true;
    setLoad({ kind: "loading" });
    getBuild(pack.id, pack.build)
      .then((build) => alive && setLoad({ kind: "ready", build }))
      .catch((e) => alive && setLoad({ kind: "error", message: String(e) }));
    return () => {
      alive = false;
    };
  }, [pack.id, pack.build]);

  const packNews = news.filter((n) => !n.pack || n.pack === pack.id);
  const onceGroups =
    load.kind === "ready" ? load.build.groups.filter((g) => g.mode === "once" && g.files > 0) : [];

  return (
    <div className="pack-view">
      <header
        className={`pack-header${background ? " with-bg" : ""}`}
        style={background ? { backgroundImage: `linear-gradient(180deg, #0f111566, #0f1115 95%), url(${background})` } : undefined}
      >
        <h1>
          {pack.name}
          {pack.channel === "beta" && <span className="badge">бета</span>}
        </h1>
        <div className="muted">
          Версия {pack.version} · build {pack.build} · Minecraft {pack.minecraft} · {pack.loader}
          {load.kind === "ready" && load.build.loaderVersion && ` ${load.build.loaderVersion}`}
        </div>
        {pack.description && <p>{pack.description}</p>}
        <div className="header-actions">
          <button className="ghost small-btn" onClick={() => setSettingsOpen(true)}>
            Настройки сборки
          </button>
          <button className="ghost small-btn" onClick={() => openInstanceDir(pack.id)}>
            Папка игры
          </button>
          <button
            className="ghost small-btn"
            onClick={() => openInstanceDir(pack.id, true)}
            title="Положи сюда свои моды (.jar) — они будут добавляться в игру и не пропадут при обновлениях"
          >
            Клиентские моды
          </button>
          {onceGroups.length > 0 && (
            <button className="ghost small-btn" disabled={busy} onClick={() => setRestoreOpen(true)}>
              Восстановить файлы…
            </button>
          )}
          <button
            className="ghost small-btn"
            disabled={busy}
            onClick={onRepair}
            title="Перепроверить все файлы игры и сборки и скачать испорченные"
          >
            Проверить и починить
          </button>
        </div>
      </header>

      {load.kind === "loading" && <div className="muted">Загрузка информации о сборке…</div>}
      {load.kind === "error" && <div className="load-error">{load.message}</div>}
      {load.kind === "ready" && (
        <>
          {load.build.changelog && (
            <section className="card">
              <h2>Что нового · {formatDate(load.build.created)}</h2>
              <Markdown text={load.build.changelog} />
            </section>
          )}
          <section className="card">
            <h2>
              Состав · {load.build.files} файлов, {formatSize(load.build.totalSize)}
            </h2>
            <table className="groups">
              <tbody>
                {load.build.groups
                  .filter((g) => g.files > 0)
                  .map((g) => (
                    <tr key={g.id}>
                      <td>{g.title}</td>
                      <td className="muted">
                        {g.mode === "sync" ? "всегда как в сборке" : "ставится один раз"}
                      </td>
                      <td className="num">{g.files}</td>
                      <td className="num">{formatSize(g.size)}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </section>
        </>
      )}

      <NewsList news={packNews} />
      {settingsOpen && (
        <PackSettingsDialog
          packName={pack.name}
          value={packSettings}
          recommendedMb={load.kind === "ready" ? load.build.memoryRecommended : null}
          totalMemoryMb={totalMemoryMb}
          onClose={() => setSettingsOpen(false)}
          onSave={(v) => {
            setSettingsOpen(false);
            onSaveSettings(v);
          }}
        />
      )}
      {restoreOpen && (
        <RestoreDialog
          groups={onceGroups}
          onClose={() => setRestoreOpen(false)}
          onConfirm={(ids) => {
            setRestoreOpen(false);
            onRestore(ids);
          }}
        />
      )}
    </div>
  );
}
