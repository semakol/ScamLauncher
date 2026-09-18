//! Загрузка файлов: проверка sha1, повторы, параллельность и зеркала.

use anyhow::{Context, Result, anyhow, bail};
use futures::{StreamExt, TryStreamExt, stream};
use reqwest::header::RANGE;
use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Ход установки для интерфейса.
#[derive(Debug, Clone)]
pub enum Progress {
    /// Новый этап («Загрузка Java», «Ресурсы игры»…).
    Stage(String),
    /// Байты текущего этапа.
    Bytes { done: u64, total: u64 },
}

pub trait ProgressSink: Fn(Progress) + Send + Sync {}
impl<T: Fn(Progress) + Send + Sync> ProgressSink for T {}

/// Официальные адреса по типам — ключи те же, что в `mirrors` в `index.json`.
const OFFICIAL: &[(&str, &[&str])] = &[
    (
        "mojang-meta",
        &[
            "https://piston-meta.mojang.com",
            "https://launchermeta.mojang.com",
        ],
    ),
    (
        "mojang-data",
        &[
            "https://piston-data.mojang.com",
            "https://launcher.mojang.com",
        ],
    ),
    ("libraries", &["https://libraries.minecraft.net"]),
    ("assets", &["https://resources.download.minecraft.net"]),
    ("fabric-meta", &["https://meta.fabricmc.net"]),
    ("fabric-maven", &["https://maven.fabricmc.net"]),
    ("quilt-meta", &["https://meta.quiltmc.org"]),
    ("quilt-maven", &["https://maven.quiltmc.org"]),
    ("forge-maven", &["https://maven.minecraftforge.net"]),
    ("neoforge-maven", &["https://maven.neoforged.net"]),
];

/// Зеркала: сначала всегда официальный адрес, потом альтернативы по порядку.
#[derive(Debug, Clone, Default)]
pub struct Mirrors {
    /// Официальный префикс → базовые адреса зеркал.
    rules: Vec<(String, Vec<String>)>,
}

impl Mirrors {
    /// Из `index.json`: `{"libraries": ["https://mirror.example/libs"], ...}`.
    pub fn from_config(map: &BTreeMap<String, Vec<String>>) -> Self {
        let mut rules = Vec::new();
        for (kind, prefixes) in OFFICIAL {
            let Some(alts) = map.get(*kind).filter(|a| !a.is_empty()) else {
                continue;
            };
            let alts: Vec<String> = alts
                .iter()
                .map(|a| a.trim_end_matches('/').to_owned())
                .collect();
            for p in *prefixes {
                rules.push((p.to_string(), alts.clone()));
            }
        }
        Self { rules }
    }

    pub fn candidates(&self, url: &str) -> Vec<String> {
        let mut out = vec![url.to_owned()];
        for (prefix, alts) in &self.rules {
            if let Some(rest) = url.strip_prefix(prefix.as_str()) {
                out.extend(alts.iter().map(|a| format!("{a}{rest}")));
                break;
            }
        }
        out
    }
}

/// Файл, который должен лежать на диске.
#[derive(Debug, Clone)]
pub struct FileSpec {
    pub url: String,
    pub path: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub executable: bool,
}

impl FileSpec {
    pub fn new(url: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            path: path.into(),
            sha1: None,
            size: None,
            executable: false,
        }
    }

    pub fn sha1(mut self, sha1: Option<&str>) -> Self {
        self.sha1 = sha1.map(str::to_owned);
        self
    }

    pub fn size(mut self, size: Option<u64>) -> Self {
        self.size = size;
        self
    }
}

#[derive(Clone)]
pub struct Net {
    http: Client,
    mirrors: Mirrors,
    parallel: usize,
}

const ATTEMPTS: u32 = 3;

impl Net {
    pub fn new(http: Client, mirrors: Mirrors) -> Self {
        Self {
            http,
            mirrors,
            parallel: 16,
        }
    }

    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        self.get_bytes_n(url, ATTEMPTS).await
    }

    async fn get_bytes_n(&self, url: &str, attempts: u32) -> Result<Vec<u8>> {
        let mut last = anyhow!("нет адресов");
        for candidate in self.mirrors.candidates(url) {
            for attempt in 0..attempts {
                match self.http.get(&candidate).send().await {
                    Ok(r) if r.status() == StatusCode::NOT_FOUND => {
                        last = anyhow!("{candidate}: не найдено (404)");
                        break;
                    }
                    Ok(r) => match r.error_for_status() {
                        Ok(r) => match r.bytes().await {
                            Ok(b) => return Ok(b.to_vec()),
                            Err(e) => last = e.into(),
                        },
                        Err(e) => last = e.into(),
                    },
                    Err(e) => last = e.into(),
                }
                backoff(attempt).await;
            }
        }
        Err(last).with_context(|| format!("не удалось скачать {url}"))
    }

    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let bytes = self.get_bytes(url).await?;
        serde_json::from_slice(&bytes).with_context(|| format!("неверный JSON: {url}"))
    }

    /// JSON, который кэшируется на диске. Если есть sha1 — файл перекачивается только при несовпадении.
    /// Без sha1 — всегда пробуем свежий, а без сети берём кэш.
    pub async fn cached_json<T: DeserializeOwned>(&self, spec: &FileSpec) -> Result<T> {
        let fresh = if spec.sha1.is_some() {
            self.ensure(spec, false).await.map(|_| ())
        } else {
            // Есть кэш — одна попытка: без сети не ждём ретраев.
            let attempts = if spec.path.exists() { 1 } else { ATTEMPTS };
            match self.get_bytes_n(&spec.url, attempts).await {
                Ok(bytes) => write_atomic(&spec.path, &bytes).await,
                Err(e) if spec.path.exists() => {
                    eprintln!("нет сети, беру из кэша {}: {e:#}", spec.path.display());
                    Ok(())
                }
                Err(e) => Err(e),
            }
        };
        fresh?;
        let bytes = tokio::fs::read(&spec.path)
            .await
            .with_context(|| format!("не удалось прочитать {}", spec.path.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("неверный JSON: {}", spec.path.display()))
    }

    /// Скачивает файл, если его нет или он не совпадает. `true` — если качали.
    pub async fn ensure(&self, spec: &FileSpec, verify: bool) -> Result<bool> {
        if is_present(spec, verify)? {
            return Ok(false);
        }
        self.download(spec, None).await?;
        Ok(true)
    }

    /// Скачивает недостающие файлы параллельно, сообщая прогресс в байтах.
    pub async fn ensure_all(
        &self,
        specs: Vec<FileSpec>,
        verify: bool,
        progress: &dyn ProgressSink,
    ) -> Result<()> {
        let missing = tokio::task::spawn_blocking(move || -> Result<Vec<FileSpec>> {
            let mut out = Vec::new();
            for s in specs {
                if !is_present(&s, verify)? {
                    out.push(s);
                }
            }
            Ok(out)
        })
        .await??;
        if missing.is_empty() {
            return Ok(());
        }
        let total: u64 = missing.iter().map(|s| s.size.unwrap_or(0)).sum();
        let done = AtomicU64::new(0);
        progress(Progress::Bytes { done: 0, total });
        let on_chunk = |n: u64| {
            let d = done.fetch_add(n, Ordering::Relaxed) + n;
            progress(Progress::Bytes {
                done: d,
                total: total.max(d),
            });
        };
        stream::iter(missing)
            .map(|spec| {
                let on_chunk = &on_chunk;
                async move { self.download(&spec, Some(on_chunk)).await }
            })
            .buffer_unordered(self.parallel)
            .try_collect::<()>()
            .await
    }

    async fn download(
        &self,
        spec: &FileSpec,
        on_chunk: Option<&(dyn Fn(u64) + Sync)>,
    ) -> Result<()> {
        let mut last = anyhow!("нет адресов");
        for candidate in self.mirrors.candidates(&spec.url) {
            for attempt in 0..ATTEMPTS {
                match self.try_download(&candidate, spec, on_chunk).await {
                    Ok(()) => return Ok(()),
                    Err(DlError::NotFound) => {
                        last = anyhow!("{candidate}: не найдено (404)");
                        break;
                    }
                    Err(DlError::Other(e)) => last = e,
                }
                backoff(attempt).await;
            }
        }
        Err(last).with_context(|| format!("не удалось скачать {}", spec.url))
    }

    async fn try_download(
        &self,
        url: &str,
        spec: &FileSpec,
        on_chunk: Option<&(dyn Fn(u64) + Sync)>,
    ) -> std::result::Result<(), DlError> {
        let dir = spec
            .path
            .parent()
            .context("нет папки")
            .map_err(DlError::Other)?;
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(DlError::other)?;
        let tmp = part_path(&spec.path);

        // Докачка — только когда есть sha1: иначе склеенный файл нечем проверить.
        let mut offset = match &spec.sha1 {
            Some(_) => tokio::fs::metadata(&tmp).await.map_or(0, |m| m.len()),
            None => 0,
        };
        if spec.size.is_some_and(|s| offset >= s) {
            let _ = tokio::fs::remove_file(&tmp).await;
            offset = 0;
        }

        let mut req = self.http.get(url);
        if offset > 0 {
            req = req.header(RANGE, format!("bytes={offset}-"));
        }
        let mut resp = req.send().await.map_err(DlError::other)?;
        match resp.status() {
            StatusCode::NOT_FOUND => return Err(DlError::NotFound),
            StatusCode::RANGE_NOT_SATISFIABLE => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(DlError::Other(anyhow!("сервер не принял докачку")));
            }
            _ => {}
        }
        resp = resp.error_for_status().map_err(DlError::other)?;

        let resumed = offset > 0 && resp.status() == StatusCode::PARTIAL_CONTENT;
        let (mut file, mut hasher, mut size) = if resumed {
            let path = tmp.clone();
            let (hasher, _) =
                tokio::task::spawn_blocking(move || scam_core::hash::sha1_state(&path))
                    .await
                    .map_err(DlError::other)?
                    .map_err(DlError::other)?;
            let file = tokio::fs::OpenOptions::new()
                .append(true)
                .open(&tmp)
                .await
                .map_err(DlError::other)?;
            if let Some(cb) = on_chunk {
                cb(offset);
            }
            (file, hasher, offset)
        } else {
            let file = tokio::fs::File::create(&tmp)
                .await
                .map_err(DlError::other)?;
            (file, Sha1::new(), 0)
        };

        // Обрыв сети — .part остаётся для докачки.
        while let Some(chunk) = resp.chunk().await.map_err(DlError::other)? {
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(DlError::other)?;
            size += chunk.len() as u64;
            if let Some(cb) = on_chunk {
                cb(chunk.len() as u64);
            }
        }
        file.flush().await.map_err(DlError::other)?;
        drop(file);

        // Не совпало — файл испорчен, докачивать его нельзя.
        let verdict = match (&spec.size, &spec.sha1) {
            (Some(expected), _) if *expected != size => {
                Err(anyhow!("размер {size} вместо {expected}"))
            }
            (_, Some(expected))
                if !hex::encode(hasher.finalize()).eq_ignore_ascii_case(expected) =>
            {
                Err(anyhow!("sha1 не совпал"))
            }
            _ => Ok(()),
        };
        if let Err(e) = verdict {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(DlError::Other(e));
        }
        set_executable(&tmp, spec.executable).map_err(DlError::other)?;
        tokio::fs::rename(&tmp, &spec.path)
            .await
            .map_err(DlError::other)?;
        Ok(())
    }
}

enum DlError {
    NotFound,
    Other(anyhow::Error),
}

impl DlError {
    fn other(e: impl Into<anyhow::Error>) -> Self {
        DlError::Other(e.into())
    }
}

async fn backoff(attempt: u32) {
    tokio::time::sleep(Duration::from_millis(400 * 2u64.pow(attempt))).await;
}

fn part_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

async fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let tmp = part_path(path);
    tokio::fs::write(&tmp, data).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

/// Файл на месте: совпадает размер, а при `verify` (или если размер неизвестен) — и sha1.
fn is_present(spec: &FileSpec, verify: bool) -> Result<bool> {
    let meta = match std::fs::metadata(&spec.path) {
        Ok(m) if m.is_file() => m,
        Ok(_) => bail!("{} — папка, а не файл", spec.path.display()),
        Err(_) => return Ok(false),
    };
    if let Some(size) = spec.size
        && meta.len() != size
    {
        return Ok(false);
    }
    if let Some(sha1) = &spec.sha1
        && (verify || spec.size.is_none())
    {
        let (actual, _) = scam_core::hash::sha1_file(&spec.path)?;
        if !actual.eq_ignore_ascii_case(sha1) {
            return Ok(false);
        }
    }
    if spec.executable {
        set_executable(&spec.path, true)?;
    }
    Ok(true)
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if executable {
        let mut perms = std::fs::metadata(path)?.permissions();
        if perms.mode() & 0o111 != 0o111 {
            perms.set_mode(perms.mode() | 0o755);
            std::fs::set_permissions(path, perms)?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_candidates() {
        let mut cfg = BTreeMap::new();
        cfg.insert(
            "libraries".to_string(),
            vec!["https://mirror.example/maven/".to_string()],
        );
        let m = Mirrors::from_config(&cfg);
        assert_eq!(
            m.candidates("https://libraries.minecraft.net/org/lwjgl/lwjgl.jar"),
            vec![
                "https://libraries.minecraft.net/org/lwjgl/lwjgl.jar",
                "https://mirror.example/maven/org/lwjgl/lwjgl.jar"
            ]
        );
        assert_eq!(
            m.candidates("https://example.com/x"),
            vec!["https://example.com/x"]
        );
        assert_eq!(
            Mirrors::default()
                .candidates("https://libraries.minecraft.net/a")
                .len(),
            1
        );
    }

    /// Мини-HTTP-сервер: первый ответ обрывается на середине, дальше — честно (с Range или без).
    async fn server(
        body: Vec<u8>,
        honor_range: bool,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            let mut n = 0;
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                n += 1;
                let mut buf = vec![0u8; 4096];
                let len = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..len]).to_string();
                let range = req
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("range: bytes=")
                            .map(|r| r.trim_end_matches('-').to_owned())
                    })
                    .and_then(|r| r.parse::<usize>().ok());
                log.lock()
                    .unwrap()
                    .push(range.map_or("full".into(), |r| format!("range {r}")));
                let (status, part) = match range {
                    Some(from) if honor_range => ("206 Partial Content", &body[from..]),
                    _ => ("200 OK", &body[..]),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    part.len()
                );
                sock.write_all(head.as_bytes()).await.unwrap();
                if n == 1 {
                    // Обрыв: половина тела и закрытие соединения.
                    let _ = sock.write_all(&part[..part.len() / 2]).await;
                } else {
                    let _ = sock.write_all(part).await;
                }
                let _ = sock.shutdown().await;
            }
        });
        (format!("http://{addr}/file"), seen)
    }

    fn body() -> Vec<u8> {
        (0..200_000u32).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn resumes_after_broken_connection() {
        let data = body();
        let (url, seen) = server(data.clone(), true).await;
        let dir = tempfile::tempdir().unwrap();
        let spec = FileSpec::new(&url, dir.path().join("f.bin"))
            .sha1(Some(&scam_core::hash::sha1_bytes(&data)))
            .size(Some(data.len() as u64));
        let net = Net::new(Client::new(), Mirrors::default());
        net.ensure(&spec, false).await.unwrap();
        assert_eq!(std::fs::read(&spec.path).unwrap(), data);
        assert_eq!(
            seen.lock().unwrap().clone(),
            vec!["full".to_string(), "range 100000".to_string()]
        );
        assert!(!part_path(&spec.path).exists());
    }

    #[tokio::test]
    async fn restarts_when_server_ignores_range() {
        let data = body();
        let (url, seen) = server(data.clone(), false).await;
        let dir = tempfile::tempdir().unwrap();
        let spec = FileSpec::new(&url, dir.path().join("f.bin"))
            .sha1(Some(&scam_core::hash::sha1_bytes(&data)))
            .size(Some(data.len() as u64));
        Net::new(Client::new(), Mirrors::default())
            .ensure(&spec, false)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&spec.path).unwrap(), data);
        assert_eq!(
            seen.lock().unwrap()[1],
            "range 100000",
            "докачку попросили, сервер отдал целиком"
        );
    }

    #[tokio::test]
    async fn corrupted_part_is_discarded() {
        let data = body();
        let (url, _) = server(data.clone(), true).await;
        let dir = tempfile::tempdir().unwrap();
        let spec = FileSpec::new(&url, dir.path().join("f.bin"))
            .sha1(Some(&scam_core::hash::sha1_bytes(&data)))
            .size(Some(data.len() as u64));
        // Испорченное начало от прошлой попытки.
        std::fs::write(part_path(&spec.path), vec![0xAAu8; 150_000]).unwrap();
        Net::new(Client::new(), Mirrors::default())
            .ensure(&spec, false)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&spec.path).unwrap(), data);
    }

    #[test]
    fn presence() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"abc").unwrap();
        let sha = "a9993e364706816aba3e25717850c26c9cd0d89d";
        let spec = |size, sha1: Option<&str>| FileSpec::new("u", &p).size(size).sha1(sha1);
        assert!(is_present(&spec(Some(3), Some(sha)), true).unwrap());
        assert!(!is_present(&spec(Some(4), Some(sha)), false).unwrap());
        assert!(!is_present(&spec(Some(3), Some(&"0".repeat(40))), true).unwrap());
        // Без verify при известном размере хэш не считается.
        assert!(is_present(&spec(Some(3), Some(&"0".repeat(40))), false).unwrap());
        // Размер неизвестен — сверяем хэш.
        assert!(!is_present(&spec(None, Some(&"0".repeat(40))), false).unwrap());
        assert!(!is_present(&FileSpec::new("u", dir.path().join("nope")), false).unwrap());
    }
}
