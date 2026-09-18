import { useCallback, useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import {
  formatSize,
  getCatalog,
  instanceInfo,
  packSettingsOf,
  storage,
  type Catalog,
  type Pack,
} from "./api";
import PackSettingsView from "./PackSettingsView";
import SettingsView from "./SettingsView";
import { PackIcon } from "./PackImage";
import { useSettings } from "./useSettings";
import { useUpdater } from "./useUpdater";
import UpdateBanner from "./UpdateBanner";
import PackView from "./PackView";
import NewsList from "./NewsList";
import Console from "./Console";
import GameNotice from "./GameNotice";
import { ConflictNotice, DoneNotice } from "./SyncNotice";
import { useGame, type GameState } from "./useGame";

type Load =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; catalog: Catalog };

const NICK_RE = /^[A-Za-z0-9_]{3,16}$/;

export default function App() {
  const [version, setVersion] = useState("");
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [selected, setSelected] = useState<string | null>(storage.get("pack"));
  const [consoleOpen, setConsoleOpen] = useState(false);
  const [view, setView] = useState<"main" | "settings" | "packSettings">("main");
  const game = useGame();
  const settings = useSettings();
  const updater = useUpdater();
  const beta = settings.info?.settings.beta ?? false;
  const [nick, setNick] = useState("");

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion("dev"));
  }, []);

  // Ник из настроек — один раз после загрузки; сохраняется с задержкой, не на каждую букву.
  const savedNick = settings.info?.settings.nick;
  useEffect(() => {
    if (savedNick !== undefined) setNick((n) => n || savedNick);
  }, [savedNick]);
  const { update: updateSettings } = settings;
  useEffect(() => {
    if (savedNick === undefined || nick === savedNick) return;
    const t = setTimeout(() => updateSettings((s) => ({ ...s, nick })), 500);
    return () => clearTimeout(t);
  }, [nick, savedNick, updateSettings]);

  const refresh = useCallback(() => {
    setLoad({ kind: "loading" });
    getCatalog(beta)
      .then((catalog) => setLoad({ kind: "ready", catalog }))
      .catch((e) => setLoad({ kind: "error", message: String(e) }));
  }, [beta]);

  const ready = settings.info !== null;
  useEffect(() => {
    if (ready) refresh();
  }, [ready, refresh]);

  function select(id: string) {
    setSelected(id);
    setView("main");
    setConsoleOpen(false);
    storage.set("pack", id);
  }

  const catalog = load.kind === "ready" ? load.catalog : null;
  const pack = catalog?.packs.find((p) => p.id === selected) ?? null;

  // Какой билд установлен: от этого зависит «Установить» / «Обновить» / «Играть».
  const [installed, setInstalled] = useState<{ pack: string; build: number | null } | null>(null);
  const packId = pack?.id;
  const gameKind = game.state.kind;
  useEffect(() => {
    if (!packId || gameKind === "preparing") return;
    let alive = true;
    instanceInfo(packId)
      .then((i) => alive && setInstalled({ pack: packId, build: i.installedBuild }))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [packId, gameKind]);
  const installedBuild = installed && installed.pack === packId ? installed.build : undefined;

  // Непрочитанные новости: запоминаем на весь сеанс, а в настройках сразу помечаем как увиденные.
  const [unseen, setUnseen] = useState<Set<string>>(new Set());
  const seen = settings.info?.settings.seenNews;
  useEffect(() => {
    if (!catalog || !seen) return;
    const fresh = catalog.news.map((n) => n.id).filter((id) => !seen.includes(id));
    if (fresh.length === 0) return;
    setUnseen((u) => new Set([...u, ...fresh]));
    updateSettings((s) => ({ ...s, seenNews: [...s.seenNews, ...fresh].slice(-300) }));
  }, [catalog, seen, updateSettings]);

  return (
    <div className="layout">
      <aside className="sidebar">
        <div className="logo">ScamLauncher</div>
        <nav className="packs">
          {load.kind === "loading" && <div className="muted">Загрузка…</div>}
          {load.kind === "error" && (
            <div className="load-error">
              <div>Не удалось загрузить сборки</div>
              <div className="muted small">{load.message}</div>
              <button className="ghost" onClick={refresh}>
                Повторить
              </button>
            </div>
          )}
          {catalog && catalog.packs.length === 0 && <div className="muted">Сборок пока нет</div>}
          {catalog?.packs.map((p) => (
            <button
              key={p.id}
              className={`pack-item${p.id === selected ? " active" : ""}`}
              onClick={() => select(p.id)}
            >
              <PackIcon sha1={p.icon} name={p.name} />
              <span className="pack-text">
              <span className="pack-name">
                {p.name}
                {p.channel === "beta" && <span className="badge">бета</span>}
                {p.removed && <span className="badge removed">удалена</span>}
                {isActive(game.state, p.id) && <span className="dot" title="Запущена" />}
              </span>
              <span className="pack-sub">
                {p.minecraft} · {p.loader}
              </span>
              </span>
            </button>
          ))}
        </nav>
        <button
          className={`ghost settings-btn${view === "settings" ? " active-btn" : ""}`}
          onClick={() => setView(view === "settings" ? "main" : "settings")}
        >
          Настройки
        </button>
        <div className="sidebar-footer">
          v{version}
          {beta && " · бета-сборки"}
        </div>
      </aside>

      <main className="main">
        <UpdateBanner updater={updater} />
        {catalog?.offline && (
          <div className="banner banner-warn">
            <div className="banner-text">
              <strong>Нет связи с сервером сборок</strong>
              <div className="notes">
                Показаны сохранённые данные. Играть можно, если сборка уже скачана.
              </div>
            </div>
            <div className="banner-actions">
              <button className="ghost" onClick={refresh}>
                Повторить
              </button>
            </div>
          </div>
        )}
        <GameNotice
          state={game.state}
          logs={game.logs}
          onShowConsole={() => setConsoleOpen(true)}
          onDismiss={game.dismiss}
        />
        {game.state.kind === "done" && game.state.task !== "install" && (
          <DoneNotice task={game.state.task} report={game.state.report} onDismiss={game.dismiss} />
        )}
        {game.lastSync && game.lastSync.pack === pack?.id && (
          <ConflictNotice report={game.lastSync.report} onDismiss={game.dismissSync} />
        )}
        <div className="content">
          {view === "settings" && settings.info ? (
            <SettingsView
              info={settings.info}
              error={settings.error}
              busy={game.state.kind === "preparing" || game.state.kind === "running"}
              update={settings.update}
              reload={async () => {
                await settings.reload();
                refresh();
              }}
              updater={updater}
              onClose={() => setView("main")}
            />
          ) : consoleOpen ? (
            <Console lines={game.logs} onClose={() => setConsoleOpen(false)} />
          ) : pack && view === "packSettings" ? (
            <PackSettingsView
              key={pack.id}
              pack={pack}
              value={packSettingsOf(settings.info?.settings, pack.id)}
              totalMemoryMb={settings.info?.totalMemoryMb ?? 8192}
              installedBuild={installedBuild ?? null}
              busy={game.state.kind === "preparing" || game.state.kind === "running"}
              onSave={(v) => settings.update((s) => ({ ...s, packs: { ...s.packs, [pack.id]: v } }))}
              onRepair={() => game.startRepair(pack.id, pack.build)}
              onRestore={(groups) => game.startRestore(pack.id, pack.build, groups)}
              onDeleted={() => {
                setInstalled({ pack: pack.id, build: null });
                // Удалённая с сервера сборка пропадает из списка, как только её удалил игрок.
                if (pack.removed) {
                  setView("main");
                  refresh();
                }
              }}
              onClose={() => setView("main")}
            />
          ) : pack ? (
            <PackView
              pack={pack}
              news={catalog?.news ?? []}
              unseen={unseen}
              statusAddress={(() => {
                const ps = packSettingsOf(settings.info?.settings, pack.id);
                return ps.serverStatus ? ps.server || pack.server || null : null;
              })()}
              onOpenSettings={() => setView("packSettings")}
            />
          ) : (
            <div className="hero">
              <h1>Добро пожаловать</h1>
              <p className="muted">Выбери сборку слева, чтобы начать играть.</p>
              {catalog && <NewsList news={catalog.news.filter((n) => !n.pack)} unseen={unseen} />}
            </div>
          )}
        </div>
        <PlayBar
          pack={pack}
          nick={nick}
          onNick={setNick}
          game={game.state}
          installedBuild={installedBuild}
          consoleOpen={consoleOpen}
          onToggleConsole={() => setConsoleOpen((v) => !v)}
          onPlay={() => {
            if (!pack) return;
            setView("main");
            if (installedBuild !== pack.build) {
              // Не установлена или устарела — сначала скачиваем, играть — следующим нажатием.
              game.startInstall(pack.id, pack.build);
              return;
            }
            updateSettings((s) => ({ ...s, nick }));
            game.start(pack.id, pack.build, nick);
          }}
          onStop={game.stop}
        />
      </main>
    </div>
  );
}

function isActive(s: GameState, pack: string) {
  return (s.kind === "preparing" || s.kind === "running") && s.pack === pack;
}

interface PlayBarProps {
  pack: Pack | null;
  nick: string;
  onNick: (v: string) => void;
  game: GameState;
  /** undefined — ещё не знаем, null — не установлена. */
  installedBuild: number | null | undefined;
  consoleOpen: boolean;
  onToggleConsole: () => void;
  onPlay: () => void;
  onStop: () => void;
}

function PlayBar({
  pack,
  nick,
  onNick,
  game,
  installedBuild,
  consoleOpen,
  onToggleConsole,
  onPlay,
  onStop,
}: PlayBarProps) {
  const mode: "install" | "update" | "play" =
    installedBuild === null ? "install" : pack && installedBuild !== undefined && installedBuild !== pack.build ? "update" : "play";
  const nickOk = NICK_RE.test(nick);
  const busy = game.kind === "preparing" || game.kind === "running";
  const here = pack !== null && isActive(game, pack.id);

  let status: React.ReactNode = null;
  if (here && game.kind === "preparing") {
    const pct = game.total > 0 ? Math.min(100, Math.round((game.done / game.total) * 100)) : null;
    status = (
      <div className="progress">
        <div className="progress-text">
          {game.stage}
          {pct !== null && ` · ${formatSize(game.done)} из ${formatSize(game.total)}`}
        </div>
        <div className="progress-track">
          <div
            className={`progress-fill${pct === null ? " indeterminate" : ""}`}
            style={pct !== null ? { width: `${pct}%` } : undefined}
          />
        </div>
      </div>
    );
  } else if (here && game.kind === "running") {
    status = <div className="muted">Игра запущена</div>;
  } else if (pack?.removed) {
    status = <div className="muted">Сборку удалили с сервера — играть можно, обновлений не будет</div>;
  } else if (pack && mode === "install") {
    status = <div className="muted">Сборка ещё не скачана</div>;
  } else if (pack && mode === "update") {
    status = <div className="muted">Доступно обновление до версии {pack.version}</div>;
  }

  let action: React.ReactNode;
  if (here && game.kind === "running") {
    action = (
      <button className="play stop" onClick={onStop}>
        Закрыть игру
      </button>
    );
  } else {
    const reason = !pack
      ? "Выбери сборку"
      : installedBuild === undefined
        ? "Проверяю…"
        : mode === "play" && !nickOk
        ? "Ник: 3–16 символов, латиница, цифры и _"
        : busy
          ? here
            ? "Идёт подготовка"
            : "Уже запущена другая сборка"
          : undefined;
    action = (
      <button className="play" disabled={reason !== undefined} title={reason} onClick={onPlay}>
        {here ? "Подготовка…" : mode === "install" ? "Установить" : mode === "update" ? "Обновить" : "Играть"}
      </button>
    );
  }

  return (
    <div className="bottom-bar">
      <div className="status">{status}</div>
      <button className={`ghost${consoleOpen ? " active-btn" : ""}`} onClick={onToggleConsole}>
        Консоль
      </button>
      <input
        className={`nick${nick && !nickOk ? " invalid" : ""}`}
        placeholder="Ник"
        maxLength={16}
        value={nick}
        onChange={(e) => onNick(e.target.value.trim())}
        title="3–16 символов: латиница, цифры, _"
      />
      {action}
    </div>
  );
}
