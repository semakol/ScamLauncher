//! Синхронизация папки игры со сборкой.
//!
//! * `sync`-группы всегда приводятся к версии сборки; лишние файлы в пределах `prune` удаляются.
//! * `once`-группы ставятся один раз; заново — при новой ревизии, по кнопке или если игрок удалил
//!   корень группы целиком. Перед перезаписью изменённые файлы копируются в `.scam/backups/`.
//! * `mods-clients/` — моды игрока: копируются в `mods/`, если не конфликтуют с модами сборки.
//! * Всё, что не описано в сборке, не трогается.

use crate::model::{BuildManifest, FileEntry, GroupMode};
use crate::paths;
use crate::pattern::Pattern;
use crate::yadisk::PublicDisk;
use anyhow::{Context, Result, bail};
use futures::{StreamExt, TryStreamExt, stream};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

pub const STATE_DIR: &str = ".scam";
pub const CLIENT_MODS_DIR: &str = "mods-clients";
const MODS_DIR: &str = "mods";
const STATE_FILE: &str = "state.json";
const KEEP_BACKUPS: usize = 10;
const PARALLEL: usize = 6;

// ---------- состояние экземпляра ----------

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct InstanceState {
    /// Последний установленный билд.
    #[serde(default)]
    pub build: Option<u64>,
    /// once-группа → установленная ревизия.
    #[serde(default)]
    pub groups: BTreeMap<String, u32>,
    /// Клиентские моды, скопированные в `mods/`: имя файла → sha1.
    #[serde(default)]
    pub client_mods: BTreeMap<String, String>,
    /// Кэш хэшей: путь → размер, время изменения, sha1.
    #[serde(default)]
    pub hashes: BTreeMap<String, CachedHash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedHash {
    pub size: u64,
    pub mtime: u64,
    pub sha1: String,
}

impl InstanceState {
    fn path(instance: &Path) -> PathBuf {
        instance.join(STATE_DIR).join(STATE_FILE)
    }

    /// Повреждённое или отсутствующее состояние — начинаем с чистого листа.
    pub fn load(instance: &Path) -> Self {
        std::fs::read(Self::path(instance))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, instance: &Path) -> Result<()> {
        let path = Self::path(instance);
        std::fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

// ---------- источник файлов ----------

/// Откуда брать объекты сборки по sha1.
pub trait ObjectSource: Sync {
    /// Скачивает объект в `dest`, возвращает sha1 скачанного.
    fn fetch(
        &self,
        sha1: &str,
        dest: &Path,
        on_chunk: &(dyn Fn(u64) + Sync),
    ) -> impl Future<Output = Result<String>> + Send;
}

/// Объекты из публичной папки на Яндекс Диске.
pub struct PublicObjects(pub PublicDisk);

impl ObjectSource for PublicObjects {
    fn fetch(
        &self,
        sha1: &str,
        dest: &Path,
        on_chunk: &(dyn Fn(u64) + Sync),
    ) -> impl Future<Output = Result<String>> + Send {
        let rel = paths::object_file(sha1);
        async move { Ok(self.0.download_to(&rel, dest, on_chunk).await?.0) }
    }
}

// ---------- параметры и отчёт ----------

#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    /// Пересчитать хэши всех файлов, не доверяя кэшу.
    pub verify: bool,
    /// once-группы, которые игрок попросил восстановить.
    pub reinstall: Vec<String>,
    /// Опциональные группы, которые игрок выключил.
    pub disabled: Vec<String>,
    /// Сколько бэкапов миров хранить; 0 — не делать бэкап перед обновлением.
    pub world_backups: usize,
}

#[derive(Debug, Clone)]
pub enum SyncProgress {
    Stage(String),
    Bytes { done: u64, total: u64 },
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub downloaded: usize,
    pub downloaded_bytes: u64,
    pub removed: Vec<String>,
    pub installed_groups: Vec<InstalledGroup>,
    pub client_copied: Vec<String>,
    pub client_removed: Vec<String>,
    pub conflicts: Vec<ClientConflict>,
    /// Папка бэкапа, если что-то перезаписывалось.
    pub backup: Option<String>,
    /// Архив с мирами, если перед обновлением делался бэкап.
    pub worlds_backup: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledGroup {
    pub id: String,
    pub title: String,
    pub reason: InstallReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallReason {
    First,
    Revision,
    Missing,
    Requested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientConflict {
    pub file: String,
    /// Совпавшие id модов (пусто — совпало имя файла).
    pub mod_ids: Vec<String>,
}

// ---------- план ----------

struct Plan {
    state: InstanceState,
    new_hashes: BTreeMap<String, CachedHash>,
    /// sha1 → размер и куда положить.
    downloads: BTreeMap<String, (u64, Vec<String>)>,
    backups: Vec<String>,
    group_updates: Vec<(String, u32)>,
    client_copy: Vec<(PathBuf, String, String)>,
    client_keep: BTreeMap<String, String>,
    client_remove: Vec<String>,
    prune: Vec<String>,
    report: SyncReport,
}

/// Группы, которые в этот раз не ставятся: опциональные, выключенные игроком.
fn disabled_groups<'a>(m: &'a BuildManifest, opts: &SyncOptions) -> HashSet<&'a str> {
    m.groups
        .iter()
        .filter(|g| g.optional && opts.disabled.iter().any(|d| d == &g.id))
        .map(|g| g.id.as_str())
        .collect()
}

fn mtime_nanos(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64)
}

/// sha1 файла с кэшем по размеру и времени изменения. `None` — файла нет.
fn file_sha1(
    path: &Path,
    key: &str,
    old: &BTreeMap<String, CachedHash>,
    new: &mut BTreeMap<String, CachedHash>,
    verify: bool,
) -> Result<Option<String>> {
    let meta = match std::fs::metadata(path) {
        Ok(m) if m.is_file() => m,
        _ => return Ok(None),
    };
    let (size, mtime) = (meta.len(), mtime_nanos(&meta));
    if !verify
        && let Some(c) = old.get(key)
        && c.size == size
        && c.mtime == mtime
    {
        new.insert(key.to_owned(), c.clone());
        return Ok(Some(c.sha1.clone()));
    }
    let (sha1, _) = crate::hash::sha1_file(path)
        .with_context(|| format!("не удалось прочитать {}", path.display()))?;
    new.insert(
        key.to_owned(),
        CachedHash {
            size,
            mtime,
            sha1: sha1.clone(),
        },
    );
    Ok(Some(sha1))
}

fn under_root(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{root}/"))
}

fn plan(instance: &Path, m: &BuildManifest, opts: &SyncOptions) -> Result<Plan> {
    let state = InstanceState::load(instance);
    let disabled = disabled_groups(m, opts);
    let active: Vec<&FileEntry> = m
        .files
        .iter()
        .filter(|f| !disabled.contains(f.group.as_str()))
        .collect();
    let mut new_hashes = BTreeMap::new();
    let mut downloads: BTreeMap<String, (u64, Vec<String>)> = BTreeMap::new();
    let mut backups = Vec::new();
    let mut group_updates = Vec::new();
    let mut report = SyncReport::default();

    let mut add_download = |f: &FileEntry| {
        downloads
            .entry(f.sha1.clone())
            .or_insert_with(|| (f.size, Vec::new()))
            .1
            .push(f.path.clone());
    };

    // sync-группы: всё как в сборке.
    for f in active.iter().copied() {
        let Some(g) = m.group(&f.group) else { continue };
        if g.mode != GroupMode::Sync {
            continue;
        }
        let current = file_sha1(
            &instance.join(&f.path),
            &f.path,
            &state.hashes,
            &mut new_hashes,
            opts.verify,
        )?;
        if current.as_deref() != Some(f.sha1.as_str()) {
            add_download(f);
        }
    }

    // once-группы.
    for g in m
        .groups
        .iter()
        .filter(|g| g.mode == GroupMode::Once && !disabled.contains(g.id.as_str()))
    {
        let files: Vec<&FileEntry> = m.files.iter().filter(|f| f.group == g.id).collect();
        let installed = state.groups.get(&g.id).copied();
        let (reason, targets): (InstallReason, Vec<&FileEntry>) = if opts.reinstall.contains(&g.id)
        {
            (InstallReason::Requested, files)
        } else {
            match installed {
                None => (InstallReason::First, files),
                Some(rev) if g.revision > rev => (InstallReason::Revision, files),
                Some(_) => {
                    let missing: Vec<&String> = g
                        .roots
                        .iter()
                        .filter(|r| !instance.join(r).exists())
                        .collect();
                    let targets: Vec<&FileEntry> = files
                        .into_iter()
                        .filter(|f| missing.iter().any(|r| under_root(&f.path, r)))
                        .collect();
                    if targets.is_empty() {
                        continue;
                    }
                    (InstallReason::Missing, targets)
                }
            }
        };
        let mut changed = false;
        for f in targets {
            let current = file_sha1(
                &instance.join(&f.path),
                &f.path,
                &state.hashes,
                &mut new_hashes,
                opts.verify,
            )?;
            if current.as_deref() == Some(f.sha1.as_str()) {
                continue;
            }
            if current.is_some() {
                backups.push(f.path.clone());
            }
            add_download(f);
            changed = true;
        }
        group_updates.push((g.id.clone(), g.revision));
        if changed || reason == InstallReason::Requested {
            report.installed_groups.push(InstalledGroup {
                id: g.id.clone(),
                title: g.title().to_owned(),
                reason,
            });
        }
    }

    // Моды игрока.
    // Выключенные опциональные моды — не часть сборки: вместо них можно поставить свои.
    let pack_paths: HashSet<String> = active.iter().map(|f| f.path.to_lowercase()).collect();
    let mut pack_ids: HashMap<&str, ()> = HashMap::new();
    for f in active.iter().copied() {
        for id in &f.mod_ids {
            pack_ids.insert(id, ());
        }
    }
    let clients_dir = instance.join(CLIENT_MODS_DIR);
    std::fs::create_dir_all(&clients_dir)?;
    let mut client_copy = Vec::new();
    let mut client_keep = BTreeMap::new();
    let mut entries: Vec<_> = std::fs::read_dir(&clients_dir)?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let Some(name) = e.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.')
            || !name.to_lowercase().ends_with(".jar")
            || !paths::is_safe_rel_path(&name)
        {
            continue;
        }
        let src = e.path();
        let key = format!("{CLIENT_MODS_DIR}/{name}");
        let Some(sha1) = file_sha1(&src, &key, &state.hashes, &mut new_hashes, opts.verify)? else {
            continue;
        };
        let target = format!("{MODS_DIR}/{name}");
        let clashing: Vec<String> = crate::modid::mod_ids_from_jar(&src)
            .into_iter()
            .filter(|id| pack_ids.contains_key(id.as_str()))
            .collect();
        if !clashing.is_empty() || pack_paths.contains(&target.to_lowercase()) {
            report.conflicts.push(ClientConflict {
                file: name,
                mod_ids: clashing,
            });
            continue;
        }
        let current = file_sha1(
            &instance.join(&target),
            &target,
            &state.hashes,
            &mut new_hashes,
            opts.verify,
        )?;
        if current.as_deref() != Some(sha1.as_str()) {
            client_copy.push((src, name.clone(), sha1.clone()));
        }
        client_keep.insert(name, sha1);
    }
    let client_remove: Vec<String> = state
        .client_mods
        .keys()
        .filter(|n| {
            !client_keep.contains_key(*n)
                && !pack_paths.contains(&format!("{MODS_DIR}/{n}").to_lowercase())
        })
        .cloned()
        .collect();

    // Лишние файлы в пределах prune.
    let expected: HashSet<String> = active
        .iter()
        .map(|f| f.path.to_lowercase())
        .chain(
            client_keep
                .keys()
                .map(|n| format!("{MODS_DIR}/{n}").to_lowercase()),
        )
        .collect();
    let mut prune = BTreeSet::new();
    for g in m.groups.iter().filter(|g| g.mode == GroupMode::Sync) {
        for p in &g.prune {
            let Ok(pattern) = Pattern::parse(p) else {
                continue;
            };
            let Some(root) = pattern.static_root() else {
                continue;
            };
            if root == STATE_DIR
                || under_root(&root, STATE_DIR)
                || under_root(&root, CLIENT_MODS_DIR)
            {
                continue;
            }
            let dir = instance.join(&root);
            if !dir.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(&dir).follow_links(false) {
                let entry = entry?;
                if !entry.file_type().is_file() && !entry.file_type().is_symlink() {
                    continue;
                }
                let Ok(rel) = entry.path().strip_prefix(instance) else {
                    continue;
                };
                let Some(rel) = rel.to_str().map(|s| s.replace('\\', "/")) else {
                    continue;
                };
                if pattern.is_match(&rel) && !expected.contains(&rel.to_lowercase()) {
                    prune.insert(rel);
                }
            }
        }
    }

    // Файлы выключенных групп убираем, даже если они вне prune (кроме совпавших с клиентскими).
    for f in m
        .files
        .iter()
        .filter(|f| disabled.contains(f.group.as_str()))
    {
        let client = f
            .path
            .strip_prefix(&format!("{MODS_DIR}/"))
            .is_some_and(|n| client_keep.contains_key(n));
        if !client && !expected.contains(&f.path.to_lowercase()) && instance.join(&f.path).is_file()
        {
            prune.insert(f.path.clone());
        }
    }

    Ok(Plan {
        state,
        new_hashes,
        downloads,
        backups,
        group_updates,
        client_copy,
        client_keep,
        client_remove,
        prune: prune.into_iter().collect(),
        report,
    })
}

// ---------- бэкап миров ----------

pub const WORLD_BACKUPS_DIR: &str = "backups/worlds";

/// Упаковывает `saves/` в `.scam/backups/worlds/<дата>-build<N>.zip`, оставляя `keep` последних.
pub async fn backup_worlds(
    instance: &Path,
    build: u64,
    keep: usize,
    progress: &(dyn Fn(SyncProgress) + Sync),
) -> Result<Option<PathBuf>> {
    let saves = instance.join("saves");
    let files: Vec<(PathBuf, String, u64)> = {
        let saves = saves.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<(PathBuf, String, u64)>> {
            if !saves.is_dir() {
                return Ok(Vec::new());
            }
            let mut out = Vec::new();
            for e in walkdir::WalkDir::new(&saves).follow_links(false) {
                let e = e?;
                if !e.file_type().is_file() || e.file_name() == "session.lock" {
                    continue;
                }
                let Some(rel) = e
                    .path()
                    .strip_prefix(saves.parent().unwrap())
                    .ok()
                    .and_then(|r| r.to_str())
                else {
                    continue;
                };
                out.push((
                    e.path().to_owned(),
                    rel.replace('\\', "/"),
                    e.metadata()?.len(),
                ));
            }
            Ok(out)
        })
        .await??
    };
    if files.is_empty() {
        return Ok(None);
    }

    progress(SyncProgress::Stage("Бэкап миров".into()));
    let dir = instance.join(STATE_DIR).join(WORLD_BACKUPS_DIR);
    std::fs::create_dir_all(&dir)?;
    let name = format!(
        "{}-build{build}.zip",
        chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")
    );
    let target = dir.join(&name);
    let tmp = dir.join(format!("{name}.part"));
    let total: u64 = files.iter().map(|f| f.2).sum();
    progress(SyncProgress::Bytes { done: 0, total });

    // Архивация идёт в отдельном потоке, прогресс — через канал.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
    let job = {
        let tmp = tmp.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let file = std::fs::File::create(&tmp)?;
            let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(1))
                .large_file(true);
            for (path, rel, size) in files {
                zip.start_file(rel, options)?;
                let mut f = std::fs::File::open(&path)?;
                std::io::copy(&mut f, &mut zip)?;
                let _ = tx.send(size);
            }
            zip.finish()?;
            Ok(())
        })
    };
    let mut done = 0;
    while let Some(n) = rx.recv().await {
        done += n;
        progress(SyncProgress::Bytes { done, total });
    }
    if let Err(e) = job.await.map_err(anyhow::Error::from).and_then(|r| r) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.context("не удалось сделать бэкап миров"));
    }
    std::fs::rename(&tmp, &target)?;

    let mut all: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "zip"))
        .collect();
    all.sort();
    while all.len() > keep {
        let _ = std::fs::remove_file(all.remove(0));
    }
    Ok(Some(target))
}

// ---------- выполнение ----------

fn make_backup(instance: &Path, files: &[String]) -> Result<Option<PathBuf>> {
    if files.is_empty() {
        return Ok(None);
    }
    let root = instance.join(STATE_DIR).join("backups");
    let dir = root.join(chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());
    for rel in files {
        let out = dir.join(rel);
        std::fs::create_dir_all(out.parent().unwrap())?;
        std::fs::copy(instance.join(rel), &out)
            .with_context(|| format!("не удалось сохранить бэкап {rel}"))?;
    }
    // Храним только последние бэкапы.
    let mut all: Vec<PathBuf> = std::fs::read_dir(&root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    all.sort();
    while all.len() > KEEP_BACKUPS {
        let old = all.remove(0);
        let _ = std::fs::remove_dir_all(old);
    }
    Ok(Some(dir))
}

fn record_hash(instance: &Path, rel: &str, sha1: &str, hashes: &mut BTreeMap<String, CachedHash>) {
    if let Ok(meta) = std::fs::metadata(instance.join(rel)) {
        hashes.insert(
            rel.to_owned(),
            CachedHash {
                size: meta.len(),
                mtime: mtime_nanos(&meta),
                sha1: sha1.to_owned(),
            },
        );
    }
}

fn place(instance: &Path, tmp: &Path, dests: &[String]) -> Result<()> {
    for rel in dests {
        let out = instance.join(rel);
        if out.is_dir() {
            bail!("на месте файла {rel} лежит папка — удали её и повтори");
        }
        std::fs::create_dir_all(out.parent().unwrap())?;
        std::fs::copy(tmp, &out).with_context(|| format!("не удалось записать {rel}"))?;
    }
    Ok(())
}

/// Приводит папку игры к сборке.
pub async fn sync(
    instance: &Path,
    manifest: &BuildManifest,
    source: &impl ObjectSource,
    opts: &SyncOptions,
    progress: &(dyn Fn(SyncProgress) + Sync),
) -> Result<SyncReport> {
    crate::remote::validate_manifest(manifest).map_err(anyhow::Error::msg)?;
    progress(SyncProgress::Stage("Проверка файлов сборки".into()));
    std::fs::create_dir_all(instance)?;
    let plan = {
        let (instance, manifest, opts) = (instance.to_owned(), manifest.clone(), opts.clone());
        tokio::task::spawn_blocking(move || plan(&instance, &manifest, &opts)).await??
    };
    let Plan {
        mut state,
        mut new_hashes,
        downloads,
        backups,
        group_updates,
        client_copy,
        client_keep,
        client_remove,
        prune,
        mut report,
    } = plan;

    // Бэкап миров — только когда сборка меняет версию, а не при первой установке.
    let updating = state.build.is_some_and(|b| b != manifest.build);
    if updating && opts.world_backups > 0 {
        report.worlds_backup =
            backup_worlds(instance, manifest.build, opts.world_backups, progress)
                .await?
                .map(|p| p.to_string_lossy().into_owned());
    }

    report.backup = make_backup(instance, &backups)?.map(|p| p.to_string_lossy().into_owned());

    if !downloads.is_empty() {
        progress(SyncProgress::Stage("Загрузка файлов сборки".into()));
        let total: u64 = downloads.values().map(|(s, _)| *s).sum();
        let done = AtomicU64::new(0);
        let on_chunk = |n: u64| {
            let d = done.fetch_add(n, Ordering::Relaxed) + n;
            progress(SyncProgress::Bytes {
                done: d,
                total: total.max(d),
            });
        };
        progress(SyncProgress::Bytes { done: 0, total });
        let tmp_dir = instance.join(STATE_DIR).join("tmp");
        std::fs::create_dir_all(&tmp_dir)?;
        let placed: Vec<(String, Vec<String>)> = stream::iter(downloads)
            .map(|(sha1, (_size, dests))| {
                let (tmp_dir, on_chunk) = (&tmp_dir, &on_chunk);
                async move {
                    let tmp = tmp_dir.join(&sha1);
                    let got = source.fetch(&sha1, &tmp, on_chunk).await?;
                    if got != sha1 {
                        let _ = std::fs::remove_file(&tmp);
                        bail!("файл сборки повреждён при загрузке ({})", dests[0]);
                    }
                    place(instance, &tmp, &dests)?;
                    let _ = std::fs::remove_file(&tmp);
                    Ok((sha1, dests))
                }
            })
            .buffer_unordered(PARALLEL)
            .try_collect()
            .await?;
        let _ = std::fs::remove_dir_all(&tmp_dir);
        for (sha1, dests) in &placed {
            for rel in dests {
                record_hash(instance, rel, sha1, &mut new_hashes);
            }
        }
        report.downloaded = placed.iter().map(|(_, d)| d.len()).sum();
        report.downloaded_bytes = total;
    }

    // Моды игрока.
    for (src, name, sha1) in &client_copy {
        let rel = format!("{MODS_DIR}/{name}");
        let out = instance.join(&rel);
        std::fs::create_dir_all(out.parent().unwrap())?;
        std::fs::copy(src, &out)
            .with_context(|| format!("не удалось скопировать клиентский мод {name}"))?;
        record_hash(instance, &rel, sha1, &mut new_hashes);
        report.client_copied.push(name.clone());
    }
    for name in &client_remove {
        let rel = format!("{MODS_DIR}/{name}");
        let path = instance.join(&rel);
        if path.is_file() {
            std::fs::remove_file(&path)?;
        }
        new_hashes.remove(&rel);
        report.client_removed.push(name.clone());
    }

    for rel in &prune {
        let path = instance.join(rel);
        if std::fs::symlink_metadata(&path).is_ok() {
            std::fs::remove_file(&path).with_context(|| format!("не удалось удалить {rel}"))?;
        }
        new_hashes.remove(rel);
    }
    report.removed = prune;

    state.build = Some(manifest.build);
    for (id, rev) in group_updates {
        state.groups.insert(id, rev);
    }
    state.client_mods = client_keep;
    state.hashes = new_hashes;
    state.save(instance)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Group, Loader, LoaderKind, Memory, SCHEMA};
    use std::io::Write;
    use std::sync::Mutex;

    /// Объекты из памяти; считает скачивания.
    struct Mem {
        objects: HashMap<String, Vec<u8>>,
        fetched: Mutex<Vec<String>>,
    }

    impl ObjectSource for Mem {
        fn fetch(
            &self,
            sha1: &str,
            dest: &Path,
            on_chunk: &(dyn Fn(u64) + Sync),
        ) -> impl Future<Output = Result<String>> + Send {
            let data = self.objects.get(sha1).cloned();
            self.fetched.lock().unwrap().push(sha1.to_owned());
            async move {
                let data = data.context("нет объекта")?;
                std::fs::write(dest, &data)?;
                on_chunk(data.len() as u64);
                Ok(crate::hash::sha1_bytes(&data))
            }
        }
    }

    fn jar(mod_id: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut buf);
        zip.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(format!(r#"{{"id":"{mod_id}"}}"#).as_bytes())
            .unwrap();
        zip.finish().unwrap();
        buf.into_inner()
    }

    fn group(
        id: &str,
        name: Option<&str>,
        mode: GroupMode,
        roots: &[&str],
        prune: &[&str],
    ) -> Group {
        Group {
            id: id.into(),
            name: name.map(Into::into),
            mode,
            revision: 1,
            roots: roots.iter().map(|s| s.to_string()).collect(),
            prune: prune.iter().map(|s| s.to_string()).collect(),
            optional: false,
            enabled_by_default: true,
            description: None,
        }
    }

    struct Pack {
        files: Vec<(String, String, Vec<u8>)>,
        groups: Vec<Group>,
        build: u64,
    }

    impl Pack {
        fn new() -> Self {
            Pack {
                files: vec![],
                groups: vec![
                    group("mods", Some("Моды"), GroupMode::Sync, &[], &["mods/*.jar"]),
                    group("controls", None, GroupMode::Once, &["options.txt"], &[]),
                    group(
                        "shaders",
                        Some("Шейдеры"),
                        GroupMode::Once,
                        &["shaderpacks"],
                        &[],
                    ),
                ],
                build: 1,
            }
        }

        fn file(mut self, path: &str, group: &str, data: &[u8]) -> Self {
            self.files.push((path.into(), group.into(), data.to_vec()));
            self
        }

        /// Опциональная группа модов — ставится ВЫШЕ общей группы mods.
        fn optional(mut self, id: &str, default: bool) -> Self {
            let mut g = group(id, Some("Миникарта"), GroupMode::Sync, &[], &[]);
            g.optional = true;
            g.enabled_by_default = default;
            self.groups.insert(0, g);
            self
        }

        fn revision(mut self, group: &str, rev: u32) -> Self {
            self.groups
                .iter_mut()
                .find(|g| g.id == group)
                .unwrap()
                .revision = rev;
            self
        }

        fn source(&self) -> Mem {
            Mem {
                objects: self
                    .files
                    .iter()
                    .map(|(_, _, d)| (crate::hash::sha1_bytes(d), d.clone()))
                    .collect(),
                fetched: Mutex::new(vec![]),
            }
        }

        fn manifest(&self) -> BuildManifest {
            BuildManifest {
                schema: SCHEMA,
                pack: "p".into(),
                name: "P".into(),
                build: self.build,
                version: "1".into(),
                created: chrono::Utc::now(),
                minecraft: "1.20.1".into(),
                loader: Loader {
                    kind: LoaderKind::Fabric,
                    version: Some("0.16".into()),
                },
                java: None,
                memory: Memory::default(),
                changelog: None,
                groups: self.groups.clone(),
                files: self
                    .files
                    .iter()
                    .map(|(p, g, d)| FileEntry {
                        path: p.clone(),
                        sha1: crate::hash::sha1_bytes(d),
                        size: d.len() as u64,
                        group: g.clone(),
                        mod_ids: if p.ends_with(".jar") {
                            let tmp = tempfile::NamedTempFile::new().unwrap();
                            std::fs::write(tmp.path(), d).unwrap();
                            crate::modid::mod_ids_from_jar(tmp.path())
                        } else {
                            vec![]
                        },
                    })
                    .collect(),
            }
        }
    }

    async fn run(dir: &Path, pack: &Pack, opts: SyncOptions) -> (SyncReport, usize) {
        let src = pack.source();
        let report = sync(dir, &pack.manifest(), &src, &opts, &|_| {})
            .await
            .unwrap();
        let n = src.fetched.lock().unwrap().len();
        (report, n)
    }

    fn read(dir: &Path, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(dir.join(rel)).ok()
    }

    fn base() -> Pack {
        Pack::new()
            .file("mods/sodium.jar", "mods", &jar("sodium"))
            .file("mods/create.jar", "mods", &jar("create"))
            .file("options.txt", "controls", b"key_jump:space")
            .file("shaderpacks/BSL.zip", "shaders", b"bsl")
            .file("shaderpacks/Complementary.zip", "shaders", b"compl")
    }

    #[tokio::test]
    async fn first_install_then_nothing_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let pack = base();
        let (r, n) = run(dir.path(), &pack, Default::default()).await;
        assert_eq!(n, 5);
        assert_eq!(r.downloaded, 5);
        assert_eq!(read(dir.path(), "options.txt").unwrap(), b"key_jump:space");
        assert!(dir.path().join(CLIENT_MODS_DIR).is_dir());
        let kinds: Vec<InstallReason> = r.installed_groups.iter().map(|g| g.reason).collect();
        assert_eq!(kinds, vec![InstallReason::First, InstallReason::First]);

        let (r, n) = run(dir.path(), &pack, Default::default()).await;
        assert_eq!(n, 0);
        assert!(r.installed_groups.is_empty() && r.removed.is_empty());
    }

    #[tokio::test]
    async fn sync_restores_and_prunes_but_once_keeps_player_changes() {
        let dir = tempfile::tempdir().unwrap();
        let pack = base();
        run(dir.path(), &pack, Default::default()).await;

        std::fs::write(dir.path().join("mods/create.jar"), b"broken").unwrap();
        std::fs::remove_file(dir.path().join("mods/sodium.jar")).unwrap();
        std::fs::write(dir.path().join("mods/cheat.jar"), b"x").unwrap();
        std::fs::write(dir.path().join("mods/readme.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("options.txt"), b"key_jump:w").unwrap();
        std::fs::remove_file(dir.path().join("shaderpacks/BSL.zip")).unwrap();
        std::fs::write(dir.path().join("my-notes.txt"), b"mine").unwrap();

        let (r, n) = run(dir.path(), &pack, Default::default()).await;
        assert_eq!(n, 2, "только два мода сборки");
        assert_eq!(read(dir.path(), "mods/create.jar").unwrap(), jar("create"));
        assert!(read(dir.path(), "mods/sodium.jar").is_some());
        assert_eq!(r.removed, vec!["mods/cheat.jar"]);
        assert!(
            read(dir.path(), "mods/readme.txt").is_some(),
            "prune только по маске mods/*.jar"
        );
        assert_eq!(
            read(dir.path(), "options.txt").unwrap(),
            b"key_jump:w",
            "настройки игрока не трогаем"
        );
        assert!(
            read(dir.path(), "shaderpacks/BSL.zip").is_none(),
            "удалённый игроком шейдер не возвращаем"
        );
        assert_eq!(read(dir.path(), "my-notes.txt").unwrap(), b"mine");
    }

    #[tokio::test]
    async fn deleted_root_is_reinstalled() {
        let dir = tempfile::tempdir().unwrap();
        let pack = base();
        run(dir.path(), &pack, Default::default()).await;
        std::fs::remove_dir_all(dir.path().join("shaderpacks")).unwrap();
        std::fs::remove_file(dir.path().join("options.txt")).unwrap();

        let (r, n) = run(dir.path(), &pack, Default::default()).await;
        assert_eq!(n, 3);
        assert!(read(dir.path(), "shaderpacks/BSL.zip").is_some());
        assert!(read(dir.path(), "options.txt").is_some());
        assert!(
            r.installed_groups
                .iter()
                .all(|g| g.reason == InstallReason::Missing)
        );
        assert!(r.backup.is_none());
    }

    #[tokio::test]
    async fn revision_bump_and_request_overwrite_with_backup() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &base(), Default::default()).await;
        std::fs::write(dir.path().join("options.txt"), b"player").unwrap();

        let pack = base().revision("controls", 2);
        let (r, _) = run(dir.path(), &pack, Default::default()).await;
        assert_eq!(read(dir.path(), "options.txt").unwrap(), b"key_jump:space");
        assert_eq!(r.installed_groups[0].reason, InstallReason::Revision);
        let backup = PathBuf::from(r.backup.unwrap());
        assert_eq!(
            std::fs::read(backup.join("options.txt")).unwrap(),
            b"player"
        );

        // Та же ревизия — больше не перезаписываем.
        std::fs::write(dir.path().join("options.txt"), b"player2").unwrap();
        run(dir.path(), &pack, Default::default()).await;
        assert_eq!(read(dir.path(), "options.txt").unwrap(), b"player2");

        // По кнопке — перезаписываем.
        let opts = SyncOptions {
            reinstall: vec!["controls".into()],
            ..Default::default()
        };
        let (r, _) = run(dir.path(), &pack, opts).await;
        assert_eq!(read(dir.path(), "options.txt").unwrap(), b"key_jump:space");
        assert_eq!(r.installed_groups[0].reason, InstallReason::Requested);
    }

    #[tokio::test]
    async fn client_mods_copy_conflict_and_removal() {
        let dir = tempfile::tempdir().unwrap();
        let pack = base();
        run(dir.path(), &pack, Default::default()).await;
        let clients = dir.path().join(CLIENT_MODS_DIR);
        std::fs::write(clients.join("minimap.jar"), jar("xaerominimap")).unwrap();
        std::fs::write(clients.join("sodium-newer.jar"), jar("sodium")).unwrap();
        std::fs::write(clients.join("create.jar"), jar("something-else")).unwrap();

        let (r, _) = run(dir.path(), &pack, Default::default()).await;
        assert_eq!(r.client_copied, vec!["minimap.jar"]);
        assert_eq!(
            read(dir.path(), "mods/minimap.jar").unwrap(),
            jar("xaerominimap")
        );
        assert!(read(dir.path(), "mods/sodium-newer.jar").is_none());
        let conflicts: Vec<(&str, Vec<String>)> = r
            .conflicts
            .iter()
            .map(|c| (c.file.as_str(), c.mod_ids.clone()))
            .collect();
        assert!(conflicts.contains(&("sodium-newer.jar", vec!["sodium".to_string()])));
        assert!(
            conflicts.contains(&("create.jar", vec![])),
            "совпало имя файла"
        );
        assert_eq!(
            read(dir.path(), "mods/create.jar").unwrap(),
            jar("create"),
            "мод сборки не перезаписан"
        );

        // Повторный запуск: клиентский мод остаётся и не удаляется prune.
        let (r, _) = run(dir.path(), &pack, Default::default()).await;
        assert!(r.removed.is_empty());
        assert!(read(dir.path(), "mods/minimap.jar").is_some());

        // Игрок убрал мод из mods-clients — пропадает и из mods.
        std::fs::remove_file(clients.join("minimap.jar")).unwrap();
        let (r, _) = run(dir.path(), &pack, Default::default()).await;
        assert_eq!(r.client_removed, vec!["minimap.jar"]);
        assert!(read(dir.path(), "mods/minimap.jar").is_none());
    }

    #[tokio::test]
    async fn pack_update_removes_old_mod_and_verify_catches_same_size_change() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &base(), Default::default()).await;

        let mut next = Pack::new()
            .file("mods/sodium.jar", "mods", &jar("sodium"))
            .file("options.txt", "controls", b"key_jump:space");
        next.build = 2;
        let (r, _) = run(dir.path(), &next, Default::default()).await;
        assert_eq!(r.removed, vec!["mods/create.jar"]);
        assert!(
            read(dir.path(), "shaderpacks/BSL.zip").is_some(),
            "once-файлы игрока остаются"
        );
        assert_eq!(InstanceState::load(dir.path()).build, Some(2));

        // Подмена с тем же размером и временем не видна по кэшу, но видна при verify.
        let path = dir.path().join("mods/sodium.jar");
        let meta = std::fs::metadata(&path).unwrap();
        let mut data = std::fs::read(&path).unwrap();
        let last = data.len() - 1;
        data[last] ^= 0xff;
        std::fs::write(&path, &data).unwrap();
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(meta.modified().unwrap()).unwrap();
        drop(f);
        let (_, n) = run(dir.path(), &next, Default::default()).await;
        assert_eq!(n, 0);
        let opts = SyncOptions {
            verify: true,
            ..Default::default()
        };
        let (_, n) = run(dir.path(), &next, opts).await;
        assert_eq!(n, 1);
        assert_eq!(read(dir.path(), "mods/sodium.jar").unwrap(), jar("sodium"));
    }

    #[tokio::test]
    async fn optional_group_can_be_disabled_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let pack = base().optional("minimap", true).file(
            "mods/minimap.jar",
            "minimap",
            &jar("xaerominimap"),
        );
        let (r, _) = run(dir.path(), &pack, Default::default()).await;
        assert!(
            read(dir.path(), "mods/minimap.jar").is_some(),
            "включена по умолчанию"
        );
        assert!(r.removed.is_empty());

        // Игрок выключил миникарту и положил свою версию того же мода.
        std::fs::write(
            dir.path().join(CLIENT_MODS_DIR).join("my-minimap.jar"),
            jar("xaerominimap"),
        )
        .unwrap();
        let off = SyncOptions {
            disabled: vec!["minimap".into()],
            ..Default::default()
        };
        let (r, n) = run(dir.path(), &pack, off.clone()).await;
        assert_eq!(n, 0);
        assert_eq!(r.removed, vec!["mods/minimap.jar"]);
        assert!(
            r.conflicts.is_empty(),
            "мод сборки выключен — своя версия не конфликтует"
        );
        assert_eq!(r.client_copied, vec!["my-minimap.jar"]);

        // Выключение обычной (не опциональной) группы игнорируется.
        let (_, n) = run(
            dir.path(),
            &pack,
            SyncOptions {
                disabled: vec!["minimap".into(), "mods".into()],
                ..Default::default()
            },
        )
        .await;
        assert_eq!(n, 0);
        assert!(read(dir.path(), "mods/sodium.jar").is_some());

        // Снова включил — мод вернулся.
        std::fs::remove_file(dir.path().join(CLIENT_MODS_DIR).join("my-minimap.jar")).unwrap();
        run(dir.path(), &pack, Default::default()).await;
        assert!(read(dir.path(), "mods/minimap.jar").is_some());
        assert!(read(dir.path(), "mods/my-minimap.jar").is_none());
    }

    #[tokio::test]
    async fn worlds_backed_up_only_on_update() {
        let dir = tempfile::tempdir().unwrap();
        let opts = SyncOptions {
            world_backups: 2,
            ..Default::default()
        };
        let (r, _) = run(dir.path(), &base(), opts.clone()).await;
        assert!(
            r.worlds_backup.is_none(),
            "первая установка — бэкапить нечего"
        );

        let world = dir.path().join("saves/Мой мир");
        std::fs::create_dir_all(world.join("region")).unwrap();
        std::fs::write(world.join("level.dat"), b"level").unwrap();
        std::fs::write(world.join("region/r.0.0.mca"), vec![7u8; 100_000]).unwrap();
        std::fs::write(world.join("session.lock"), b"").unwrap();

        let (r, _) = run(dir.path(), &base(), opts.clone()).await;
        assert!(r.worlds_backup.is_none(), "та же версия — без бэкапа");

        let mut archives = Vec::new();
        for build in 2..=4 {
            let mut next = base();
            next.build = build;
            let (r, _) = run(dir.path(), &next, opts.clone()).await;
            archives.push(PathBuf::from(r.worlds_backup.expect("обновление — бэкап")));
            std::thread::sleep(std::time::Duration::from_millis(1100)); // разные имена по секундам
        }
        let zip_path = archives.last().unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(zip_path).unwrap()).unwrap();
        let mut names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["saves/Мой мир/level.dat", "saves/Мой мир/region/r.0.0.mca"]
        );
        let left = std::fs::read_dir(zip_path.parent().unwrap())
            .unwrap()
            .count();
        assert_eq!(left, 2, "хранятся только последние");

        // Выключено — бэкапа нет.
        let mut next = base();
        next.build = 9;
        let (r, _) = run(dir.path(), &next, Default::default()).await;
        assert!(r.worlds_backup.is_none());
    }

    #[tokio::test]
    async fn corrupted_download_is_rejected() {
        struct Liar;
        impl ObjectSource for Liar {
            fn fetch(
                &self,
                _sha1: &str,
                dest: &Path,
                _on_chunk: &(dyn Fn(u64) + Sync),
            ) -> impl Future<Output = Result<String>> + Send {
                let dest = dest.to_owned();
                async move {
                    std::fs::write(&dest, b"evil")?;
                    Ok(crate::hash::sha1_bytes(b"evil"))
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let err = sync(
            dir.path(),
            &base().manifest(),
            &Liar,
            &Default::default(),
            &|_| {},
        )
        .await
        .unwrap_err();
        assert!(format!("{err:#}").contains("повреждён"));
        assert!(read(dir.path(), "mods/sodium.jar").is_none());
    }
}
