import type { SyncReport } from "./api";
import { formatSize } from "./api";

/** Итог «Проверить и починить» / «Восстановить файлы». */
export function DoneNotice({
  task,
  report,
  onDismiss,
}: {
  task: "repair" | "restore";
  report: SyncReport;
  onDismiss: () => void;
}) {
  const parts: string[] = [];
  if (report.downloaded > 0) {
    parts.push(`скачано файлов: ${report.downloaded} (${formatSize(report.downloadedBytes)})`);
  }
  if (report.removed.length > 0) parts.push(`удалено лишних: ${report.removed.length}`);
  if (report.worldsBackup) parts.push("миры сохранены в бэкап");
  if (report.installedGroups.length > 0) {
    parts.push(`восстановлено: ${report.installedGroups.map((g) => g.title).join(", ")}`);
  }
  const title = task === "repair" ? "Проверка завершена" : "Файлы сборки восстановлены";
  return (
    <div className="banner">
      <div className="banner-text">
        <strong>{title}</strong>
        <div className="notes">
          {parts.length > 0 ? parts.join(" · ") : "Всё в порядке, ничего чинить не пришлось."}
          {report.backup && "\nПрежние файлы сохранены в бэкап (.scam/backups)."}
        </div>
      </div>
      <div className="banner-actions">
        <button className="ghost" onClick={onDismiss}>
          Закрыть
        </button>
      </div>
    </div>
  );
}

/** Предупреждение о клиентских модах, которые не попали в игру. */
export function ConflictNotice({ report, onDismiss }: { report: SyncReport; onDismiss: () => void }) {
  if (report.conflicts.length === 0) return null;
  return (
    <div className="banner banner-warn">
      <div className="banner-text">
        <strong>Некоторые клиентские моды не подключены</strong>
        <div className="notes">
          {report.conflicts
            .map((c) =>
              c.modIds.length > 0
                ? `${c.file} — мод ${c.modIds.join(", ")} уже есть в сборке`
                : `${c.file} — файл с таким именем уже есть в сборке`,
            )
            .join("\n")}
        </div>
      </div>
      <div className="banner-actions">
        <button className="ghost" onClick={onDismiss}>
          Понятно
        </button>
      </div>
    </div>
  );
}
