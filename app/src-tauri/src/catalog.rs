//! Каталог сборок с Яндекс Диска для интерфейса.

use chrono::{DateTime, Utc};
use scam_core::model::BuildManifest;
use scam_core::model::{Channel, GroupMode};
use scam_core::remote::CachedStore;
use scam_mc::GameDirs;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackDto {
    id: String,
    name: String,
    description: Option<String>,
    minecraft: String,
    loader: &'static str,
    channel: Channel,
    build: u64,
    version: String,
    has_beta: bool,
    updated: DateTime<Utc>,
    /// Автор удалил сборку с сервера; у игрока она ещё установлена.
    removed: bool,
    icon: Option<String>,
    background: Option<String>,
    server: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsDto {
    id: String,
    date: DateTime<Utc>,
    title: String,
    text: String,
    pack: Option<String>,
}

#[derive(Serialize)]
pub struct CatalogDto {
    packs: Vec<PackDto>,
    news: Vec<NewsDto>,
    /// Сервер недоступен — показана сохранённая копия.
    offline: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDto {
    id: String,
    title: String,
    mode: GroupMode,
    files: usize,
    size: u64,
    optional: bool,
    enabled_by_default: bool,
    description: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildDto {
    pack: String,
    build: u64,
    version: String,
    created: DateTime<Utc>,
    minecraft: String,
    loader: &'static str,
    loader_version: Option<String>,
    changelog: Option<String>,
    memory_recommended: Option<u32>,
    files: usize,
    total_size: u64,
    groups: Vec<GroupDto>,
}

pub async fn catalog(
    store: &CachedStore,
    dirs: &GameDirs,
    beta: bool,
) -> Result<CatalogDto, String> {
    let (index, news) = tokio::join!(store.index(), store.news());
    let index = index.map_err(|e| e.to_string())?;
    let offline = index.offline;
    let index = index.value;
    // Новости не критичны: без них каталог всё равно показываем.
    let news = news.map(|n| n.value.items).unwrap_or_default();

    let packs: Vec<PackDto> = index
        .packs
        .iter()
        .filter_map(|p| {
            let (channel, info) = p.resolve(beta)?;
            Some(PackDto {
                id: p.id.clone(),
                name: p.name.clone(),
                description: p.description.clone(),
                minecraft: p.minecraft.clone(),
                loader: p.loader.title(),
                channel,
                build: info.build,
                version: info.version.clone(),
                has_beta: p.channels.beta.is_some(),
                updated: p.updated,
                removed: false,
                icon: p.icon.as_ref().map(|o| o.sha1.clone()),
                background: p.background.as_ref().map(|o| o.sha1.clone()),
                server: p.server.clone(),
            })
        })
        .collect();
    let mut packs: Vec<PackDto> = packs;
    packs.extend(removed_packs(&index, dirs));
    let news = news
        .into_iter()
        .map(|n| NewsDto {
            id: n.id,
            date: n.date,
            title: n.title,
            text: n.text,
            pack: n.pack,
        })
        .collect();
    Ok(CatalogDto {
        packs,
        news,
        offline,
    })
}

/// Установленные у игрока сборки, которых больше нет на сервере.
fn removed_packs(index: &scam_core::model::Index, dirs: &GameDirs) -> Vec<PackDto> {
    let Ok(entries) = std::fs::read_dir(dirs.root.join("instances")) else {
        return Vec::new();
    };
    let mut out: Vec<PackDto> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let id = e.file_name().to_str()?.to_owned();
            if !scam_core::paths::is_valid_id(&id) || index.pack(&id).is_some() {
                return None;
            }
            let dir = e.path();
            let build = scam_core::sync::InstanceState::load(&dir).build?;
            let p = crate::local::load_pack(&dir)?;
            let m = crate::local::load_manifest(&dir).filter(|m| m.build == build)?;
            Some(PackDto {
                id,
                name: p.name,
                description: p.description,
                minecraft: m.minecraft.clone(),
                loader: m.loader.kind.title(),
                channel: Channel::Stable,
                build,
                version: m.version.clone(),
                has_beta: false,
                updated: p.updated,
                removed: true,
                icon: p.icon.map(|o| o.sha1),
                background: p.background.map(|o| o.sha1),
                server: p.server,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Описание билда; если на сервере его уже нет — из копии в папке сборки.
pub async fn build(
    store: &CachedStore,
    dirs: &GameDirs,
    pack: &str,
    build: u64,
) -> Result<BuildDto, String> {
    let m = match store.build(pack, build).await {
        Ok(m) => m,
        Err(e) => crate::local::load_manifest(&dirs.instance(pack))
            .filter(|m| m.build == build)
            .ok_or_else(|| e.to_string())?,
    };
    Ok(build_dto(&m))
}

fn build_dto(m: &BuildManifest) -> BuildDto {
    let mut per: HashMap<&str, (usize, u64)> = HashMap::new();
    for f in &m.files {
        let e = per.entry(f.group.as_str()).or_default();
        e.0 += 1;
        e.1 += f.size;
    }
    let groups = m
        .groups
        .iter()
        .map(|g| {
            let (files, size) = per.get(g.id.as_str()).copied().unwrap_or_default();
            GroupDto {
                id: g.id.clone(),
                title: g.title().to_owned(),
                mode: g.mode,
                files,
                size,
                optional: g.optional,
                enabled_by_default: g.enabled_by_default,
                description: g.description.clone(),
            }
        })
        .collect();
    BuildDto {
        pack: m.pack.clone(),
        build: m.build,
        version: m.version.clone(),
        created: m.created,
        minecraft: m.minecraft.clone(),
        loader: m.loader.kind.title(),
        loader_version: m.loader.version.clone(),
        changelog: m.changelog.clone(),
        memory_recommended: m.memory.recommended,
        files: m.files.len(),
        total_size: m.total_size(),
        groups,
    }
}
