//! Настройки лаунчера: файл в папке настроек приложения (не в папке игры — её можно перенести).

use scam_mc::GameDirs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Своя папка игры; `None` — стандартная.
    pub game_dir: Option<PathBuf>,
    pub beta: bool,
    pub nick: String,
    /// Дополнительные JVM-аргументы для всех сборок.
    pub jvm_args: String,
    pub packs: BTreeMap<String, PackSettings>,
    /// id новостей, которые игрок уже видел.
    pub seen_news: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PackSettings {
    /// Память в МБ; `None` — рекомендованная сборкой.
    pub memory_mb: Option<u32>,
    pub jvm_args: String,
    /// Свой адрес сервера; пусто — из сборки.
    pub server: String,
    /// Показывать статус сервера (по умолчанию да).
    pub server_status: bool,
    /// Сразу заходить на сервер при запуске (по умолчанию нет).
    pub auto_connect: bool,
    /// Выбор игрока по опциональным модам: id группы → включена.
    pub optional: BTreeMap<String, bool>,
    /// Бэкап миров перед обновлением сборки (по умолчанию да).
    pub backup_worlds: bool,
}

impl Default for PackSettings {
    fn default() -> Self {
        Self {
            memory_mb: None,
            jvm_args: String::new(),
            server: String::new(),
            server_status: true,
            auto_connect: false,
            optional: BTreeMap::new(),
            backup_worlds: true,
        }
    }
}

pub struct SettingsStore {
    path: PathBuf,
    inner: Mutex<Settings>,
}

impl SettingsStore {
    pub fn load(path: PathBuf) -> Self {
        let inner = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            path,
            inner: Mutex::new(inner),
        }
    }

    pub fn get(&self) -> Settings {
        self.inner.lock().unwrap().clone()
    }

    pub fn save(&self, settings: Settings) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())?;
        *self.inner.lock().unwrap() = settings;
        Ok(())
    }

    pub fn dirs(&self) -> GameDirs {
        GameDirs::new(
            self.inner
                .lock()
                .unwrap()
                .game_dir
                .clone()
                .unwrap_or_else(GameDirs::default_root),
        )
    }
}

/// Разбивает строку аргументов как оболочка: пробелы, кавычки "…" и '…'.
pub fn split_args(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                has = true;
            }
            None if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            None => cur.push(c),
        }
    }
    if quote.is_some() {
        return Err("незакрытая кавычка в JVM-аргументах".into());
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

/// Объём оперативной памяти компьютера, МБ.
pub fn total_memory_mb() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.total_memory() / 1024 / 1024
}

/// Переносит папку игры. Если на одном диске — мгновенно, иначе копирует и удаляет старую.
pub fn move_dir(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        let empty = std::fs::read_dir(to)
            .map_err(|e| e.to_string())?
            .next()
            .is_none();
        if !empty {
            return Err("Новая папка не пустая — выбери пустую".into());
        }
        std::fs::remove_dir(to).map_err(|e| e.to_string())?;
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_tree(from, to).map_err(|e| {
        let _ = std::fs::remove_dir_all(to);
        format!("не удалось перенести файлы: {e}")
    })?;
    std::fs::remove_dir_all(from)
        .map_err(|e| format!("файлы скопированы, но старая папка не удалилась: {e}"))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in walkdir::WalkDir::new(from).follow_links(false) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(from).unwrap();
        let out = to.join(rel);
        let ft = entry.file_type();
        if ft.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else if ft.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(entry.path())?, &out)?;
        } else {
            std::fs::copy(entry.path(), &out)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        assert_eq!(split_args("").unwrap(), Vec::<String>::new());
        assert_eq!(
            split_args(" -Xss2M  -Dfoo=\"a b\" '-Dbar=c d' ").unwrap(),
            vec!["-Xss2M", "-Dfoo=a b", "-Dbar=c d"]
        );
        assert_eq!(split_args("-Dx=\"\"").unwrap(), vec!["-Dx="]);
        assert!(split_args("-Dx=\"oops").is_err());
    }

    #[test]
    fn memory_is_detected() {
        assert!(total_memory_mb() >= 1024);
    }

    #[test]
    fn move_and_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let from = tmp.path().join("a");
        std::fs::create_dir_all(from.join("x/y")).unwrap();
        std::fs::write(from.join("x/y/f"), b"1").unwrap();
        let to = tmp.path().join("b/c");
        move_dir(&from, &to).unwrap();
        assert_eq!(std::fs::read(to.join("x/y/f")).unwrap(), b"1");
        assert!(!from.exists());

        let busy = tmp.path().join("busy");
        std::fs::create_dir_all(&busy).unwrap();
        std::fs::write(busy.join("z"), b"").unwrap();
        assert!(move_dir(&to, &busy).is_err());

        let dst = tmp.path().join("copy");
        copy_tree(&to, &dst).unwrap();
        assert_eq!(std::fs::read(dst.join("x/y/f")).unwrap(), b"1");
    }
}
