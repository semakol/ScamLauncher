//! Каталог сборок с Яндекс Диска для интерфейса.

use chrono::{DateTime, Utc};
use scam_core::model::{Channel, GroupMode};
use scam_core::remote::CachedStore;
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
    icon: Option<String>,
    background: Option<String>,
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

pub async fn catalog(store: &CachedStore, beta: bool) -> Result<CatalogDto, String> {
    let (index, news) = tokio::join!(store.index(), store.news());
    let index = index.map_err(|e| e.to_string())?;
    let offline = index.offline;
    let index = index.value;
    // Новости не критичны: без них каталог всё равно показываем.
    let news = news.map(|n| n.value.items).unwrap_or_default();

    let packs = index
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
                icon: p.icon.as_ref().map(|o| o.sha1.clone()),
                background: p.background.as_ref().map(|o| o.sha1.clone()),
            })
        })
        .collect();
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

pub async fn build(store: &CachedStore, pack: &str, build: u64) -> Result<BuildDto, String> {
    let m = store.build(pack, build).await.map_err(|e| e.to_string())?;
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
            }
        })
        .collect();
    Ok(BuildDto {
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
    })
}
