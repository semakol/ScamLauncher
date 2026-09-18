import { useCallback, useEffect, useState } from "react";
import { getSettings, saveSettings, storage, type Settings, type SettingsInfo } from "./api";

/** Настройки хранятся в бэкенде; здесь — копия для интерфейса. */
export function useSettings() {
  const [info, setInfo] = useState<SettingsInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const loaded = await getSettings();
      // Ник и бета раньше жили в localStorage — переносим один раз.
      const s = loaded.settings;
      const oldNick = storage.get("nick");
      if (!s.nick && oldNick) {
        s.nick = oldNick;
        s.beta = storage.get("beta") === "1";
        await saveSettings(s).catch(() => {});
      }
      setInfo(loaded);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  const update = useCallback(
    async (patch: (s: Settings) => Settings) => {
      if (!info) return;
      const next = patch(structuredClone(info.settings));
      setInfo({ ...info, settings: next });
      try {
        await saveSettings(next);
        setError(null);
      } catch (e) {
        setError(String(e));
      }
    },
    [info],
  );

  return { info, error, update, reload };
}
