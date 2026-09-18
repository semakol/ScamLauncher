//! Форматы файлов на Яндекс Диске: `index.json`, `news.json`, `packs/<id>/builds/<N>.json`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

/// Версия формата. Лаунчер отказывается читать файлы с большей версией.
pub const SCHEMA: u32 = 1;

/// Корневой каталог сборок.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub schema: u32,
    #[serde(default)]
    pub packs: Vec<IndexPack>,
    /// Скрытые сборки: лаунчер их не показывает, но билды и файлы на месте.
    /// Отдельным списком, а не флагом — чтобы их не показывали и старые версии лаунчера.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<IndexPack>,
    /// Зеркала: тип источника (`mojang-meta`, `libraries`, …) → базовые URL по приоритету.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mirrors: BTreeMap<String, Vec<String>>,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            packs: Vec::new(),
            hidden: Vec::new(),
            mirrors: BTreeMap::new(),
        }
    }
}

impl Index {
    /// Сборка по id — и видимая, и скрытая.
    pub fn pack(&self, id: &str) -> Option<&IndexPack> {
        self.all_packs().find(|p| p.id == id)
    }

    pub fn pack_mut(&mut self, id: &str) -> Option<&mut IndexPack> {
        self.packs
            .iter_mut()
            .chain(self.hidden.iter_mut())
            .find(|p| p.id == id)
    }

    /// Все сборки на диске, включая скрытые.
    pub fn all_packs(&self) -> impl Iterator<Item = &IndexPack> {
        self.packs.iter().chain(self.hidden.iter())
    }

    pub fn is_hidden(&self, id: &str) -> bool {
        self.hidden.iter().any(|p| p.id == id)
    }

    /// Прячет или показывает сборку. `false` — сборки нет или она уже в нужном состоянии.
    pub fn set_hidden(&mut self, id: &str, hidden: bool) -> bool {
        let (from, to) = if hidden {
            (&mut self.packs, &mut self.hidden)
        } else {
            (&mut self.hidden, &mut self.packs)
        };
        match from.iter().position(|p| p.id == id) {
            Some(i) => {
                to.push(from.remove(i));
                true
            }
            None => false,
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.packs.retain(|p| p.id != id);
        self.hidden.retain(|p| p.id != id);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexPack {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub minecraft: String,
    pub loader: LoaderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ObjectRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<ObjectRef>,
    /// Адрес сервера сборки (`mc.example.com` или `host:port`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default)]
    pub channels: Channels,
    pub updated: DateTime<Utc>,
}

impl IndexPack {
    /// Какой билд показывать игроку. С включённой бетой берётся бета,
    /// если она новее релиза. Сборки только с бетой видны лишь бета-тестерам.
    pub fn resolve(&self, beta_enabled: bool) -> Option<(Channel, &ChannelInfo)> {
        let stable = self.channels.stable.as_ref();
        let beta = self.channels.beta.as_ref();
        match (stable, beta) {
            (Some(s), Some(b)) if beta_enabled && b.build > s.build => Some((Channel::Beta, b)),
            (Some(s), _) => Some((Channel::Stable, s)),
            (None, Some(b)) if beta_enabled => Some((Channel::Beta, b)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Channels {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stable: Option<ChannelInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beta: Option<ChannelInfo>,
}

impl Channels {
    pub fn get(&self, channel: Channel) -> Option<&ChannelInfo> {
        match channel {
            Channel::Stable => self.stable.as_ref(),
            Channel::Beta => self.beta.as_ref(),
        }
    }

    pub fn set(&mut self, channel: Channel, info: ChannelInfo) {
        match channel {
            Channel::Stable => self.stable = Some(info),
            Channel::Beta => self.beta = Some(info),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub build: u64,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Beta,
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Channel::Stable => "релиз",
            Channel::Beta => "бета",
        })
    }
}

/// Ссылка на объект в `objects/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub sha1: String,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
    Vanilla,
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl LoaderKind {
    pub fn title(self) -> &'static str {
        match self {
            LoaderKind::Vanilla => "Vanilla",
            LoaderKind::Fabric => "Fabric",
            LoaderKind::Quilt => "Quilt",
            LoaderKind::Forge => "Forge",
            LoaderKind::NeoForge => "NeoForge",
        }
    }
}

impl FromStr for LoaderKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "vanilla" => Ok(Self::Vanilla),
            "fabric" => Ok(Self::Fabric),
            "quilt" => Ok(Self::Quilt),
            "forge" => Ok(Self::Forge),
            "neoforge" => Ok(Self::NeoForge),
            _ => Err(format!(
                "неизвестный загрузчик «{s}» (vanilla, fabric, quilt, forge, neoforge)"
            )),
        }
    }
}

/// Полное описание одного билда сборки. Файл неизменяемый: после публикации не переписывается.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildManifest {
    pub schema: u32,
    pub pack: String,
    pub name: String,
    pub build: u64,
    pub version: String,
    pub created: DateTime<Utc>,
    pub minecraft: String,
    pub loader: Loader,
    /// Переопределение мажорной версии Java (по умолчанию — из метаданных Mojang).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java: Option<u32>,
    #[serde(default)]
    pub memory: Memory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<String>,
    pub groups: Vec<Group>,
    pub files: Vec<FileEntry>,
}

impl BuildManifest {
    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loader {
    pub kind: LoaderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Память в мегабайтах.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupMode {
    /// Всегда приводится к версии сборки.
    Sync,
    /// Ставится один раз; заново — если пропал корень группы или выросла ревизия.
    Once,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub mode: GroupMode,
    /// Для `once`: увеличение заставляет лаунчер перезаписать группу (с бэкапом).
    #[serde(default = "default_revision")]
    pub revision: u32,
    /// Для `once`: папки/файлы, пропажа которых означает «группу удалили — поставить заново».
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    /// Для `sync`: маски, в пределах которых лишние файлы удаляются.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prune: Vec<String>,
    /// Опциональная группа: игрок может её выключить.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    /// Для опциональной: включена ли, пока игрок не решил сам.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled_by_default: bool,
    /// Пояснение для игрока (показывается у опциональных групп).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Group {
    pub fn title(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

fn default_revision() -> u32 {
    1
}

fn yes() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Путь относительно папки игры, через `/`.
    pub path: String,
    pub sha1: String,
    pub size: u64,
    pub group: String,
    /// id модов внутри jar (для проверки конфликтов с клиентскими модами).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mod_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct News {
    pub schema: u32,
    #[serde(default)]
    pub items: Vec<NewsItem>,
}

impl Default for News {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub id: String,
    pub date: DateTime<Utc>,
    pub title: String,
    pub text: String,
    /// Если задано — новость относится к конкретной сборке.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(stable: Option<u64>, beta: Option<u64>) -> IndexPack {
        let info = |b| ChannelInfo {
            build: b,
            version: format!("v{b}"),
        };
        IndexPack {
            id: "p".into(),
            name: "P".into(),
            description: None,
            minecraft: "1.20.1".into(),
            loader: LoaderKind::Fabric,
            icon: None,
            background: None,
            server: None,
            channels: Channels {
                stable: stable.map(info),
                beta: beta.map(info),
            },
            updated: Utc::now(),
        }
    }

    #[test]
    fn resolve_channels() {
        let p = pack(Some(5), Some(7));
        assert_eq!(p.resolve(false).unwrap().1.build, 5);
        assert_eq!(
            p.resolve(true).unwrap(),
            (Channel::Beta, &p.channels.beta.clone().unwrap())
        );

        // Бета старше релиза — даже бета-тестеры получают релиз.
        let p = pack(Some(8), Some(7));
        assert_eq!(p.resolve(true).unwrap().0, Channel::Stable);

        // Только бета — скрыта от обычных игроков.
        let p = pack(None, Some(3));
        assert!(p.resolve(false).is_none());
        assert_eq!(p.resolve(true).unwrap().1.build, 3);

        assert!(pack(None, None).resolve(true).is_none());
    }

    #[test]
    fn hide_and_show() {
        let mut index = Index::default();
        index.packs.push(pack(Some(1), None));
        assert!(index.set_hidden("p", true));
        assert!(!index.set_hidden("p", true));
        assert!(index.packs.is_empty() && index.is_hidden("p"));
        assert!(index.pack("p").is_some(), "скрытая сборка находится по id");
        assert_eq!(index.all_packs().count(), 1);

        // Старый лаунчер (без поля hidden) скрытую сборку не увидит.
        #[derive(serde::Deserialize)]
        struct OldIndex {
            packs: Vec<serde_json::Value>,
        }
        let old: OldIndex = serde_json::from_str(&serde_json::to_string(&index).unwrap()).unwrap();
        assert!(old.packs.is_empty());

        assert!(index.set_hidden("p", false));
        assert_eq!(index.packs.len(), 1);
        index.remove("p");
        assert_eq!(index.all_packs().count(), 0);
    }

    #[test]
    fn loader_serde() {
        assert_eq!(
            serde_json::to_string(&LoaderKind::NeoForge).unwrap(),
            "\"neoforge\""
        );
        assert_eq!(
            "NeoForge".parse::<LoaderKind>().unwrap(),
            LoaderKind::NeoForge
        );
        assert!("optifine".parse::<LoaderKind>().is_err());
    }

    #[test]
    fn group_revision_defaults_to_one() {
        let g: Group = serde_json::from_str(r#"{"id":"x","mode":"once"}"#).unwrap();
        assert_eq!(g.revision, 1);
    }
}
