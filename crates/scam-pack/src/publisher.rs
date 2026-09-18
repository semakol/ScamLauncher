//! Запись в папку сборок на диске владельца.

use anyhow::{Context, Result, bail};
use scam_core::paths;
use scam_core::remote::Store;
use scam_core::yadisk::{Disk, public_link_id};
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;
use tokio::sync::Mutex;

/// Находит путь опубликованной папки из `scam.config.json` на диске владельца токена.
pub async fn resolve_root(disk: &Disk) -> Result<String> {
    let url = &scam_core::config::project().yandex.public_url;
    let want = public_link_id(url).with_context(|| format!("странная публичная ссылка: {url}"))?;
    let dirs = disk.public_dirs().await?;
    dirs.into_iter()
        .find(|r| r.public_url.as_deref().and_then(public_link_id) == Some(want))
        .map(|r| r.path)
        .with_context(|| {
            format!(
                "на этом аккаунте нет опубликованной папки {url}. \
                 Войди в аккаунт, которому принадлежит папка, и проверь, что доступ по ссылке включён"
            )
        })
}

pub struct Publisher {
    pub disk: Disk,
    pub root: String,
    made_dirs: Mutex<HashSet<String>>,
}

impl Publisher {
    pub async fn connect(http: reqwest::Client) -> Result<Self> {
        let token = crate::auth::require_token()?;
        let disk = Disk::new(http, token);
        let root = resolve_root(&disk).await?;
        Ok(Self {
            disk,
            root,
            made_dirs: Mutex::new(HashSet::new()),
        })
    }

    pub fn store(&self) -> Store {
        Store::Private {
            disk: self.disk.clone(),
            root: self.root.clone(),
        }
    }

    pub fn path(&self, rel: &str) -> String {
        Store::private_path(&self.root, rel)
    }

    /// Создаёт папку и всех родителей (относительно корня).
    pub async fn mkdir_p(&self, rel_dir: &str) -> Result<()> {
        let mut cur = String::new();
        for part in rel_dir.split('/') {
            if !cur.is_empty() {
                cur.push('/');
            }
            cur.push_str(part);
            if self.made_dirs.lock().await.contains(&cur) {
                continue;
            }
            self.disk.mkdir(&self.path(&cur)).await?;
            self.made_dirs.lock().await.insert(cur.clone());
        }
        Ok(())
    }

    pub async fn object_exists(&self, sha1: &str, size: u64) -> Result<bool> {
        let res = self
            .disk
            .resource(&self.path(&paths::object_file(sha1)))
            .await?;
        Ok(res.is_some_and(|r| r.size == Some(size)))
    }

    pub async fn upload_object(&self, sha1: &str, file: &Path) -> Result<()> {
        self.mkdir_p(&paths::object_dir(sha1)).await?;
        self.disk
            .upload_file(&self.path(&paths::object_file(sha1)), file, true)
            .await
            .with_context(|| format!("не удалось залить {}", file.display()))
    }

    pub async fn put_json<T: Serialize>(
        &self,
        rel: &str,
        value: &T,
        overwrite: bool,
    ) -> Result<()> {
        let mut data = serde_json::to_vec_pretty(value)?;
        data.push(b'\n');
        if let Some((dir, _)) = rel.rsplit_once('/') {
            self.mkdir_p(dir).await?;
        }
        self.disk
            .upload_bytes(&self.path(rel), &data, overwrite)
            .await
            .with_context(|| format!("не удалось записать {rel}"))
    }

    /// Номера всех билдов сборки на диске.
    pub async fn build_numbers(&self, pack: &str) -> Result<Vec<u64>> {
        let items = match self
            .disk
            .list_dir(&self.path(&paths::builds_dir(pack)))
            .await
        {
            Ok(items) => items,
            Err(e) if e.is_not_found() => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut nums: Vec<u64> = items
            .iter()
            .filter_map(|r| r.name.strip_suffix(".json")?.parse().ok())
            .collect();
        nums.sort_unstable();
        Ok(nums)
    }

    pub async fn delete(&self, rel: &str, permanently: bool) -> Result<()> {
        if rel.is_empty() || rel.contains("..") {
            bail!("отказ удалять «{rel}»");
        }
        Ok(self.disk.delete(&self.path(rel), permanently).await?)
    }
}
