//! Чтение каталога сборок: через публичную ссылку (лаунчер) или с токеном (`scam-pack`).

use crate::model::{BuildManifest, Index, News, SCHEMA};
use crate::paths::{self, INDEX_FILE, NEWS_FILE};
use crate::yadisk::{Disk, PublicDisk, YaError};
use serde::de::DeserializeOwned;

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
