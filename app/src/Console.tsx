import { useEffect, useRef } from "react";
import { copyText, formatLog, type LogLine } from "./api";

export default function Console({ lines, onClose }: { lines: LogLine[]; onClose: () => void }) {
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  useEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  return (
    <div className="console">
      <div className="console-head">
        <span>Консоль игры</span>
        <div className="console-actions">
          <button className="ghost small-btn" onClick={() => copyText(formatLog(lines))}>
            Копировать
          </button>
          <button className="ghost small-btn" onClick={onClose}>
            Закрыть
          </button>
        </div>
      </div>
      <div
        className="console-body"
        ref={box}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
      >
        {lines.length === 0 && <div className="muted">Здесь появится вывод игры</div>}
        {lines.map((l, i) => (
          <div key={i} className={`log-line level-${(l.level ?? "none").toLowerCase()}`}>
            {l.level && <span className="log-level">{l.level}</span>}
            {l.text}
          </div>
        ))}
      </div>
    </div>
  );
}
