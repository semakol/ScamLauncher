import type { ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * Небольшой безопасный Markdown: заголовки, списки, абзацы, **жирный**, *курсив*, `код`, [ссылки](https://…).
 * HTML не вставляется — всё строится из React-элементов.
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
      blocks.push(<p key={blocks.length}>{inline(para.join(" "))}</p>);
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

const INLINE = /(\*\*[^*]+\*\*|\*[^*]+\*|`[^`]+`|\[[^\]]+\]\([^)\s]+\))/g;

function inline(text: string): ReactNode[] {
  return text.split(INLINE).map((part, i) => {
    if (part.startsWith("**") && part.endsWith("**") && part.length > 4) {
      return <strong key={i}>{part.slice(2, -2)}</strong>;
    }
    if (part.startsWith("`") && part.endsWith("`") && part.length > 2) {
      return <code key={i}>{part.slice(1, -1)}</code>;
    }
    const link = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(part);
    if (link) {
      const url = link[2];
      if (/^https?:\/\//i.test(url)) {
        return (
          <a
            key={i}
            href={url}
            onClick={(e) => {
              e.preventDefault();
              openUrl(url).catch(() => {});
            }}
          >
            {link[1]}
          </a>
        );
      }
      return link[1];
    }
    if (part.startsWith("*") && part.endsWith("*") && part.length > 2) {
      return <em key={i}>{part.slice(1, -1)}</em>;
    }
    return part;
  });
}
