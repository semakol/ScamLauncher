import { useCallback, useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { getCatalog, storage, type Catalog } from "./api";
import UpdateBanner from "./UpdateBanner";
import PackView from "./PackView";
import NewsList from "./NewsList";

type Load =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; catalog: Catalog };

const NICK_RE = /^[A-Za-z0-9_]{3,16}$/;

export default function App() {
  const [version, setVersion] = useState("");
  const [beta, setBeta] = useState(storage.get("beta") === "1");
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [selected, setSelected] = useState<string | null>(storage.get("pack"));
  const [nick, setNick] = useState(storage.get("nick") ?? "");

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion("dev"));
  }, []);

  const refresh = useCallback(() => {
    setLoad({ kind: "loading" });
    getCatalog(beta)
      .then((catalog) => setLoad({ kind: "ready", catalog }))
      .catch((e) => setLoad({ kind: "error", message: String(e) }));
  }, [beta]);

  useEffect(refresh, [refresh]);

  function select(id: string) {
    setSelected(id);
    storage.set("pack", id);
  }

  function toggleBeta(on: boolean) {
    setBeta(on);
    storage.set("beta", on ? "1" : "0");
  }

  function changeNick(value: string) {
    setNick(value);
    storage.set("nick", value);
  }

  const catalog = load.kind === "ready" ? load.catalog : null;
  const pack = catalog?.packs.find((p) => p.id === selected) ?? null;
  const nickOk = NICK_RE.test(nick);

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
              <span className="pack-name">
                {p.name}
                {p.channel === "beta" && <span className="badge">бета</span>}
              </span>
              <span className="pack-sub">
                {p.minecraft} · {p.loader}
              </span>
            </button>
          ))}
        </nav>
        <label className="toggle">
          <input type="checkbox" checked={beta} onChange={(e) => toggleBeta(e.target.checked)} />
          Бета-версии сборок
        </label>
        <div className="sidebar-footer">v{version}</div>
      </aside>

      <main className="main">
        <UpdateBanner />
        <div className="content">
          {pack ? (
            <PackView pack={pack} news={catalog?.news ?? []} />
          ) : (
            <div className="hero">
              <h1>Добро пожаловать</h1>
              <p className="muted">Выбери сборку слева, чтобы начать играть.</p>
              {catalog && <NewsList news={catalog.news.filter((n) => !n.pack)} />}
            </div>
          )}
        </div>
        <div className="bottom-bar">
          <input
            className={`nick${nick && !nickOk ? " invalid" : ""}`}
            placeholder="Ник"
            maxLength={16}
            value={nick}
            onChange={(e) => changeNick(e.target.value.trim())}
            title="3–16 символов: латиница, цифры, _"
          />
          <button className="play" disabled title="Запуск игры появится в следующей версии">
            Играть
          </button>
        </div>
      </main>
    </div>
  );
}
