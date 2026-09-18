import { formatDate, type NewsItem } from "./api";
import Markdown from "./Markdown";

/** Новости; непрочитанные — с золотой рамкой. */
export default function NewsList({ news, unseen }: { news: NewsItem[]; unseen: Set<string> }) {
  if (news.length === 0) return null;
  return (
    <section className="news">
      <h2>Новости</h2>
      {news.map((n) => (
        <article key={n.id} className={`card${unseen.has(n.id) ? " news-unseen" : ""}`}>
          <div className="muted small">
            {formatDate(n.date)}
            {unseen.has(n.id) && <span className="new-badge">новое</span>}
          </div>
          <h3>{n.title}</h3>
          <Markdown text={n.text} />
        </article>
      ))}
    </section>
  );
}
