import { useEffect, useState } from "react";
import { formatDate, formatSize, getBuild, type BuildInfo, type NewsItem, type Pack } from "./api";
import NewsList from "./NewsList";

type Load =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; build: BuildInfo };

export default function PackView({ pack, news }: { pack: Pack; news: NewsItem[] }) {
  const [load, setLoad] = useState<Load>({ kind: "loading" });

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
      <header className="pack-header">
        <h1>
          {pack.name}
          {pack.channel === "beta" && <span className="badge">бета</span>}
        </h1>
        <div className="muted">
          Версия {pack.version} · build {pack.build} · Minecraft {pack.minecraft} · {pack.loader}
          {load.kind === "ready" && load.build.loaderVersion && ` ${load.build.loaderVersion}`}
        </div>
        {pack.description && <p>{pack.description}</p>}
      </header>

      {load.kind === "loading" && <div className="muted">Загрузка информации о сборке…</div>}
      {load.kind === "error" && <div className="load-error">{load.message}</div>}
      {load.kind === "ready" && (
        <>
          {load.build.changelog && (
            <section className="card">
              <h2>Что нового · {formatDate(load.build.created)}</h2>
              <pre className="changelog">{load.build.changelog}</pre>
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
    </div>
  );
}
