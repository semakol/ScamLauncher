import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  install,
  killGame,
  play,
  repair,
  restore,
  runningPack,
  type GameEvent,
  type LogLine,
  type DoneTask,
  type SyncReport,
} from "./api";

export type GameState =
  | { kind: "idle" }
  | { kind: "preparing"; pack: string; stage: string; done: number; total: number }
  | { kind: "running"; pack: string }
  | {
      kind: "exited";
      pack: string;
      code: number | null;
      crashed: boolean;
      killed: boolean;
      crashReport: string | null;
    }
  | { kind: "failed"; pack: string; message: string }
  | { kind: "done"; pack: string; task: DoneTask; report: SyncReport };

const MAX_LINES = 5000;

/** Состояние запуска и лог игры — живут в бэкенде, сюда приходят событиями. */
export function useGame() {
  const [state, setState] = useState<GameState>({ kind: "idle" });
  const [logs, setLogs] = useState<LogLine[]>([]);
  /** Отчёт синхронизации перед последним запуском (для предупреждений о клиентских модах). */
  const [lastSync, setLastSync] = useState<{ pack: string; report: SyncReport } | null>(null);
  const logsRef = useRef<LogLine[]>([]);

  useEffect(() => {
    // После перезагрузки окна игра может уже идти.
    runningPack()
      .then((pack) => pack && setState({ kind: "running", pack }))
      .catch(() => {});

    const unlisten = listen<GameEvent>("game", ({ payload: e }) => {
      switch (e.kind) {
        case "stage":
          setState((s) => ({
            kind: "preparing",
            pack: e.pack,
            stage: e.text,
            done: 0,
            total: s.kind === "preparing" && s.stage === e.text ? s.total : 0,
          }));
          break;
        case "bytes":
          setState((s) =>
            s.kind === "preparing" ? { ...s, done: e.done, total: e.total } : s,
          );
          break;
        case "synced":
          setLastSync({ pack: e.pack, report: e.report });
          break;
        case "done":
          setState({ kind: "done", pack: e.pack, task: e.task, report: e.report });
          break;
        case "started":
          setState({ kind: "running", pack: e.pack });
          break;
        case "log": {
          const next = logsRef.current.concat(e.lines);
          logsRef.current = next.length > MAX_LINES ? next.slice(-MAX_LINES) : next;
          setLogs(logsRef.current);
          break;
        }
        case "exited":
          setState({ ...e, kind: "exited" });
          break;
        case "failed":
          setState({ kind: "failed", pack: e.pack, message: e.message });
          break;
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const run = useCallback(async (pack: string, call: () => Promise<void>, clearLogs: boolean) => {
    if (clearLogs) {
      logsRef.current = [];
      setLogs([]);
    }
    setState({ kind: "preparing", pack, stage: "Подготовка", done: 0, total: 0 });
    try {
      await call();
    } catch (e) {
      setState({ kind: "failed", pack, message: String(e) });
    }
  }, []);

  const start = useCallback(
    (pack: string, build: number, nick: string) => run(pack, () => play(pack, build, nick), true),
    [run],
  );
  const startInstall = useCallback(
    (pack: string, build: number) => run(pack, () => install(pack, build), false),
    [run],
  );
  const startRepair = useCallback(
    (pack: string, build: number) => run(pack, () => repair(pack, build), false),
    [run],
  );
  const startRestore = useCallback(
    (pack: string, build: number, groups: string[]) =>
      run(pack, () => restore(pack, build, groups), false),
    [run],
  );

  const stop = useCallback(() => killGame().catch(() => false), []);
  const dismiss = useCallback(() => setState({ kind: "idle" }), []);
  const dismissSync = useCallback(() => setLastSync(null), []);

  return {
    state,
    logs,
    lastSync,
    start,
    startInstall,
    startRepair,
    startRestore,
    stop,
    dismiss,
    dismissSync,
  };
}
