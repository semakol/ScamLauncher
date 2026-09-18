import { useEffect, useState } from "react";
import { serverStatus, type ServerStatus as Status } from "./api";

type State = { kind: "loading" } | { kind: "online"; status: Status } | { kind: "offline" };

/** Статус сервера в карточке сборки; обновляется раз в 30 секунд. */
export default function ServerStatus({ address }: { address: string }) {
  const [state, setState] = useState<State>({ kind: "loading" });

  useEffect(() => {
    let alive = true;
    setState({ kind: "loading" });
    const load = () =>
      serverStatus(address)
        .then((status) => alive && setState({ kind: "online", status }))
        .catch(() => alive && setState({ kind: "offline" }));
    load();
    const timer = setInterval(load, 30_000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [address]);

  return (
    <div className={`server-status ${state.kind}`}>
      <span className="server-dot" />
      <span className="server-address">{address}</span>
      {state.kind === "loading" && <span className="muted">проверяю…</span>}
      {state.kind === "offline" && <span className="muted">недоступен</span>}
      {state.kind === "online" && (
        <>
          <span>
            {state.status.online} / {state.status.max}
          </span>
          <span className="muted">{state.status.latencyMs} мс</span>
          {state.status.version && <span className="muted">{state.status.version}</span>}
        </>
      )}
      {state.kind === "online" && state.status.motd && (
        <div className="server-motd">{state.status.motd}</div>
      )}
    </div>
  );
}
