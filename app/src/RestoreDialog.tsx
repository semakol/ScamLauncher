import { useState } from "react";
import { formatSize, type GroupInfo } from "./api";

interface Props {
  groups: GroupInfo[];
  onConfirm: (ids: string[]) => void;
  onClose: () => void;
}

/** Выбор once-групп, которые нужно вернуть к версии сборки. */
export default function RestoreDialog({ groups, onConfirm, onClose }: Props) {
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const toggle = (id: string) =>
    setPicked((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Восстановить файлы сборки</h2>
        <p className="muted">
          Отмеченное вернётся к версии сборки. Твои текущие файлы сохранятся в бэкап — их можно будет
          достать из папки игры (.scam/backups).
        </p>
        <div className="restore-list">
          {groups.map((g) => (
            <label key={g.id} className="restore-item">
              <input type="checkbox" checked={picked.has(g.id)} onChange={() => toggle(g.id)} />
              <span className="restore-title">{g.title}</span>
              <span className="muted small">
                {g.files} файлов · {formatSize(g.size)}
              </span>
            </label>
          ))}
        </div>
        <div className="modal-actions">
          <button className="ghost" onClick={onClose}>
            Отмена
          </button>
          <button disabled={picked.size === 0} onClick={() => onConfirm([...picked])}>
            Восстановить
          </button>
        </div>
      </div>
    </div>
  );
}
