import type { ReactNode } from "react";
import { openLink } from "./api";

/**
 * Небольшой безопасный Markdown: заголовки, списки, абзацы, **жирный**, *курсив*, `код`,
 * [ссылки](https://…) и просто адреса https://…. HTML не вставляется — только React-элементы.
 */
export default function Markdown({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  let list: string[] = [];
  let para: string[] = [];

  const flushList = () => {
    if (list.length) {
      blocks.push(
        <ul key={blocks.length}>
          {list.map((item, i) => (
            <li key={i}>{inline(item)}</li>
          ))}
        </ul>,
      );
      list = [];
    }
  };
  const flushPara = () => {
    if (para.length) {
      const lines = para;
      blocks.push(
        <p key={blocks.length}>
          {lines.map((l, i) => (
            <span key={i}>
              {i > 0 && <br />}
              {inline(l)}
            </span>
          ))}
        </p>,
      );
      para = [];
    }
  };

  for (const raw of text.split("\n")) {
    const line = raw.trimEnd();
    const heading = /^(#{1,3})\s+(.*)$/.exec(line);
    const item = /^\s*[-*+]\s+(.*)$/.exec(line);
    if (heading) {
      flushList();
      flushPara();
      const level = heading[1].length;
      const content = inline(heading[2]);
      blocks.push(
        level === 1 ? (
          <h3 key={blocks.length}>{content}</h3>
        ) : level === 2 ? (
          <h4 key={blocks.length}>{content}</h4>
        ) : (
          <h5 key={blocks.length}>{content}</h5>
        ),
      );
    } else if (item) {
      flushPara();
      list.push(item[1]);
    } else if (line.trim() === "") {
      flushList();
      flushPara();
    } else {
      flushList();
      para.push(line.trim());
    }
  }
  flushList();
  flushPara();
  return <div className="md">{blocks}</div>;
}

function Link({ url, children }: { url: string; children: ReactNode }) {
  return (
    <a
      href={url}
      title={url}
      onClick={(e) => {
        e.preventDefault();
        openLink(url).catch((err) => console.warn("Не удалось открыть ссылку:", err));
      }}
    >
      {children}
    </a>
  );
}

const INLINE = /(\*\*[^*]+\*\*|\*[^*\s][^*]*\*|`[^`]+`|\[[^\]]+\]\([^)\s]+\)|https?:\/\/[^\s<>()]+)/g;

function inline(text: string): ReactNode[] {
  return text.split(INLINE).map((part, i) => {
    if (!part) return null;
    if (part.startsWith("**") && part.endsWith("**") && part.length > 4) {
      return <strong key={i}>{inline(part.slice(2, -2))}</strong>;
    }
    if (part.startsWith("`") && part.endsWith("`") && part.length > 2) {
      return <code key={i}>{part.slice(1, -1)}</code>;
    }
    const link = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(part);
    if (link) {
      return /^https?:\/\//i.test(link[2]) ? (
        <Link key={i} url={link[2]}>
          {link[1]}
        </Link>
      ) : (
        link[1]
      );
    }
    if (/^https?:\/\//i.test(part)) {
      // Точка или запятая в конце — это конец предложения, а не часть адреса.
      const m = /^(.*?)([.,;:!?]*)$/.exec(part)!;
      return (
        <span key={i}>
          <Link url={m[1]}>{m[1]}</Link>
          {m[2]}
        </span>
      );
    }
    if (part.startsWith("*") && part.endsWith("*") && part.length > 2) {
      return <em key={i}>{part.slice(1, -1)}</em>;
    }
    return part;
  });
}
