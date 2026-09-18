import { useState } from "react";
import type { PackSettings } from "./api";

interface Props {
  packName: string;
  value: PackSettings;
  recommendedMb: number | null;
  totalMemoryMb: number;
  onSave: (v: PackSettings) => void;
  onClose: () => void;
}

const STEP = 512;
const gb = (mb: number) => `${(mb / 1024).toFixed(mb % 1024 === 0 ? 0 : 1)} ГБ`;

export default function PackSettingsDialog({
  packName,
  value,
  recommendedMb,
  totalMemoryMb,
  onSave,
  onClose,
}: Props) {
  const fallback = recommendedMb ?? 4096;
  const max = Math.max(2048, Math.floor((totalMemoryMb - 1024) / STEP) * STEP);
  const [memory, setMemory] = useState<number | null>(value.memoryMb);
  const [jvm, setJvm] = useState(value.jvmArgs);
  const current = Math.min(memory ?? fallback, max);
  const tooMuch = current > totalMemoryMb * 0.75;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Настройки: {packName}</h2>

        <label className="field">
          <span>
            Память: <strong>{gb(current)}</strong>
            {memory === null && " (как рекомендует сборка)"}
          </span>
          <input
            type="range"
            min={1024}
            max={max}
            step={STEP}
            value={current}
            onChange={(e) => setMemory(Number(e.target.value))}
          />
          <span className="muted small range-legend">
            <span>1 ГБ</span>
            <span>в компьютере {gb(totalMemoryMb)}</span>
            <span>{gb(max)}</span>
          </span>
        </label>
        {recommendedMb !== null && (
          <p className="muted small">
            Сборка рекомендует {gb(recommendedMb)}.{" "}
            {memory !== null && (
              <a
                href="#"
                onClick={(e) => {
                  e.preventDefault();
                  setMemory(null);
                }}
              >
                Вернуть
              </a>
            )}
          </p>
        )}
        {tooMuch && (
          <p className="warn small">
            Это больше 75% памяти компьютера — системе и браузеру может не хватить, игра будет тормозить.
          </p>
        )}

        <label className="field">
          <span>Дополнительные JVM-аргументы</span>
          <input
            className="text"
            value={jvm}
            placeholder="обычно не нужно"
            onChange={(e) => setJvm(e.target.value)}
          />
        </label>

        <div className="modal-actions">
          <button className="ghost" onClick={onClose}>
            Отмена
          </button>
          <button onClick={() => onSave({ memoryMb: memory, jvmArgs: jvm.trim() })}>Сохранить</button>
        </div>
      </div>
    </div>
  );
}
