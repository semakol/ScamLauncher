//! Чтение каталога сборок: через публичную ссылку (лаунчер) или с токеном (`scam-pack`).

use crate::model::{BuildManifest, Index, News, SCHEMA};
use crate::paths::{self, INDEX_FILE, NEWS_FILE};
use crate::yadisk::{Disk, PublicDisk, YaError};
use serde::de::DeserializeOwned;
use std::future::Future;

#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error(transparent)]
    Disk(#[from] YaError),
    #[error("{file} повреждён: {err}")]
    Json {
        file: String,
        err: serde_json::Error,
    },
    #[error(
        "{file} в формате v{found}, а эта версия понимает только до v{SCHEMA} — обнови лаунчер"
    )]
    Schema { file: String, found: u32 },
    #[error("{file}: {msg}")]
    Invalid { file: String, msg: String },
}

pub type Result<T> = std::result::Result<T, RemoteError>;

#[derive(Clone)]
pub enum Store {
    Public(PublicDisk),
    /// `root` — путь опубликованной папки на диске владельца, например `disk:/ScamLauncher`.
    Private {
        disk: Disk,
        root: String,
    },
}

impl Store {
    pub fn private_path(root: &str, rel: &str) -> String {
        format!("{}/{rel}", root.trim_end_matches('/'))
    }

    pub async fn get_bytes(&self, rel: &str) -> std::result::Result<Vec<u8>, YaError> {
        match self {
            Store::Public(d) => d.get_bytes(rel).await,
            Store::Private { disk, root } => disk.get_bytes(&Self::private_path(root, rel)).await,
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, rel: &str) -> Result<Option<T>> {
        let bytes = match self.get_bytes(rel).await {
            Ok(b) => b,
            Err(e) if e.is_not_found() => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|err| RemoteError::Json {
                file: rel.into(),
                err,
            })
    }

    fn check_schema(file: &str, found: u32) -> Result<()> {
        if found > SCHEMA {
            return Err(RemoteError::Schema {
                file: file.into(),
                found,
            });
        }
        Ok(())
    }

    /// `index.json`; если его ещё нет — пустой каталог.
    pub async fn index(&self) -> Result<Index> {
        let index: Index = self.get_json(INDEX_FILE).await?.unwrap_or_default();
        Self::check_schema(INDEX_FILE, index.schema)?;
        Ok(index)
    }

    pub async fn news(&self) -> Result<News> {
        let news: News = self.get_json(NEWS_FILE).await?.unwrap_or_default();
        Self::check_schema(NEWS_FILE, news.schema)?;
        Ok(news)
    }

    /// Манифест билда с проверкой путей и хэшей.
    pub async fn build(&self, pack: &str, build: u64) -> Result<BuildManifest> {
        let file = paths::build_file(pack, build);
        let manifest: BuildManifest = self
            .get_json(&file)
            .await?
            .ok_or_else(|| RemoteError::Disk(YaError::NotFound(file.clone())))?;
        Self::check_schema(&file, manifest.schema)?;
        validate_manifest(&manifest).map_err(|msg| RemoteError::Invalid { file, msg })?;
        Ok(manifest)
    }
}

impl RemoteError {
    pub fn is_network(&self) -> bool {
        matches!(self, RemoteError::Disk(e) if e.is_network())
    }
}

/// Значение и признак «взято из сохранённой копии, сервер недоступен».
#[derive(Debug, Clone)]
pub struct Cached<T> {
    pub value: T,
    pub offline: bool,
}

/// Каталог с копией на диске: без сети отдаёт последнюю удачную версию.
/// Манифесты билдов неизменяемы — сохранённый берётся сразу, без запроса.
#[derive(Clone)]
pub struct CachedStore {
    store: Store,
    dir: std::path::PathBuf,
}

impl CachedStore {
    pub fn new(store: Store, dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            store,
            dir: dir.into(),
        }
    }

    fn save<T: serde::Serialize>(&self, rel: &str, value: &T) {
        let path = self.dir.join(rel);
        let tmp = path.with_extension("tmp");
        let ok = path
            .parent()
            .is_some_and(|d| std::fs::create_dir_all(d).is_ok())
            && serde_json::to_vec(value).is_ok_and(|b| std::fs::write(&tmp, b).is_ok())
            && std::fs::rename(&tmp, &path).is_ok();
        if !ok {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn load<T: DeserializeOwned>(&self, rel: &str) -> Option<T> {
        serde_json::from_slice(&std::fs::read(self.dir.join(rel)).ok()?).ok()
    }

    async fn cached<T, F>(&self, rel: &str, fetch: F) -> Result<Cached<T>>
    where
        T: DeserializeOwned + serde::Serialize,
        F: Future<Output = Result<T>>,
    {
        match fetch.await {
            Ok(value) => {
                self.save(rel, &value);
                Ok(Cached {
                    value,
                    offline: false,
                })
            }
            Err(e) if e.is_network() => match self.load(rel) {
                Some(value) => Ok(Cached {
                    value,
                    offline: true,
                }),
                None => Err(e),
            },
            Err(e) => Err(e),
        }
    }

    pub async fn index(&self) -> Result<Cached<Index>> {
        self.cached(INDEX_FILE, self.store.index()).await
    }

    pub async fn news(&self) -> Result<Cached<News>> {
        self.cached(NEWS_FILE, self.store.news()).await
    }

    pub async fn build(&self, pack: &str, build: u64) -> Result<BuildManifest> {
        let rel = paths::build_file(pack, build);
        if let Some(m) = self.load::<BuildManifest>(&rel)
            && m.pack == pack
            && m.build == build
            && validate_manifest(&m).is_ok()
        {
            return Ok(m);
        }
        let m = self.store.build(pack, build).await?;
        self.save(&rel, &m);
        Ok(m)
    }
}

/// Всё, что лаунчер потом запишет на диск игрока, должно пройти эту проверку.
pub fn validate_manifest(m: &BuildManifest) -> std::result::Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for f in &m.files {
        if !paths::is_safe_rel_path(&f.path) {
            return Err(format!("небезопасный путь «{}»", f.path));
        }
        if !paths::is_sha1(&f.sha1) {
            return Err(format!("неверный sha1 у «{}»", f.path));
        }
        if m.group(&f.group).is_none() {
            return Err(format!(
                "файл «{}» ссылается на неизвестную группу «{}»",
                f.path, f.group
            ));
        }
        if !seen.insert(f.path.to_lowercase()) {
            return Err(format!("файл «{}» указан дважды", f.path));
        }
    }
    for g in &m.groups {
        if let Some(bad) = g.roots.iter().find(|r| !paths::is_safe_rel_path(r)) {
            return Err(format!("небезопасный корень группы «{bad}»"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Channels, IndexPack, LoaderKind};

    /// Клиент, у которого нет сети: все запросы уходят в закрытый порт.
    fn offline_store() -> Store {
        let http = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .build()
            .unwrap();
        Store::Public(PublicDisk::new(http, "https://disk.yandex.ru/d/x").with_attempts(1))
    }

    #[tokio::test]
    async fn offline_falls_back_to_saved_copy() {
        let dir = tempfile::tempdir().unwrap();
        let store = CachedStore::new(offline_store(), dir.path());

        // Копии нет — честная ошибка сети.
        let err = store.index().await.unwrap_err();
        assert!(err.is_network());

        // Есть сохранённая копия — отдаём её с пометкой offline.
        let mut index = Index::default();
        index.packs.push(IndexPack {
            id: "p".into(),
            name: "P".into(),
            description: None,
            minecraft: "1.20.1".into(),
            loader: LoaderKind::Fabric,
            icon: None,
            background: None,
            server: None,
            channels: Channels::default(),
            updated: chrono::Utc::now(),
        });
        store.save(INDEX_FILE, &index);
        let got = store.index().await.unwrap();
        assert!(got.offline);
        assert_eq!(got.value.packs[0].id, "p");
    }
}
