import { useEffect, useState } from "react";
import { formatDate, getBuild, type BuildInfo, type NewsItem, type Pack } from "./api";
import NewsList from "./NewsList";
import Markdown from "./Markdown";
import { useImage } from "./PackImage";
import ServerStatus from "./ServerStatus";

type Load =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; build: BuildInfo };

interface Props {
  pack: Pack;
  news: NewsItem[];
  unseen: Set<string>;
  /** Адрес сервера, если статус нужно показывать. */
  statusAddress: string | null;
  onOpenSettings: () => void;
}

/** Карточка сборки: шапка, «Что нового» и сразу новости. Всё остальное — в настройках сборки. */
export default function PackView({ pack, news, unseen, statusAddress, onOpenSettings }: Props) {
  const [load, setLoad] = useState<Load>({ kind: "loading" });
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

  return (
    <div className="pack-view">
      <header
        className={`pack-header${background ? " with-bg" : ""}`}
        style={
          background
            ? { backgroundImage: `linear-gradient(180deg, #0f111566, #0f1115 95%), url(${background})` }
            : undefined
        }
      >
        <div className="pack-title-row">
          <h1>
            {pack.name}
            {pack.channel === "beta" && <span className="badge">бета</span>}
          </h1>
          <button className="ghost small-btn" onClick={onOpenSettings}>
            Настройки сборки
          </button>
        </div>
        <div className="muted">
          Версия {pack.version} · build {pack.build} · Minecraft {pack.minecraft} · {pack.loader}
          {load.kind === "ready" && load.build.loaderVersion && ` ${load.build.loaderVersion}`}
        </div>
        {pack.description && <p>{pack.description}</p>}
        {pack.removed && (
          <div className="removed-note">
            Автор удалил эту сборку с сервера. Она останется у тебя, пока ты сам её не удалишь: играть можно,
            но обновлений и починки больше не будет.
          </div>
        )}
        {statusAddress && <ServerStatus address={statusAddress} />}
      </header>

      {load.kind === "error" && <div className="load-error">{load.message}</div>}
      {load.kind === "ready" && load.build.changelog && (
        <section className="card">
          <h2>Что нового · {formatDate(load.build.created)}</h2>
          <Markdown text={load.build.changelog} />
        </section>
      )}

      <NewsList news={packNews} unseen={unseen} />
    </div>
  );
}
