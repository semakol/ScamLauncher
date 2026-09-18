import { formatDate, type NewsItem } from "./api";

export default function NewsList({ news }: { news: NewsItem[] }) {
  if (news.length === 0) return null;
  return (
    <section className="news">
      <h2>Новости</h2>
      {news.map((n) => (
        <article key={n.id} className="card">
          <div className="muted small">{formatDate(n.date)}</div>
          <h3>{n.title}</h3>
          <p className="news-text">{n.text}</p>
        </article>
      ))}
    </section>
  );
}
