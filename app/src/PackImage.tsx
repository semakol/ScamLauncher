import { useEffect, useState } from "react";
import { getImage } from "./api";

/** data URL картинки сборки или null, пока грузится / если её нет. */
export function useImage(sha1: string | null): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    setUrl(null);
    if (sha1) {
      getImage(sha1)
        .then((u) => alive && setUrl(u))
        .catch(() => {});
    }
    return () => {
      alive = false;
    };
  }, [sha1]);
  return url;
}

/** Иконка сборки; без картинки — первая буква названия. */
export function PackIcon({ sha1, name, size = 32 }: { sha1: string | null; name: string; size?: number }) {
  const url = useImage(sha1);
  return url ? (
    <img className="pack-icon" src={url} width={size} height={size} alt="" />
  ) : (
    <span className="pack-icon placeholder" style={{ width: size, height: size }}>
      {name.slice(0, 1).toUpperCase()}
    </span>
  );
}
