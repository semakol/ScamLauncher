use crate::packfile::{self, InitOptions, Pack};
use crate::publisher::Publisher;
use crate::scan::{self, Scan};
use crate::ui;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use console::style;
use futures::{StreamExt, TryStreamExt, stream};
use scam_core::model::{
    BuildManifest, Channel, ChannelInfo, FileEntry, Index, IndexPack, Loader, LoaderKind, Memory,
    NewsItem, ObjectRef, SCHEMA,
};
use scam_core::paths;
use scam_core::remote::Store;
use scam_core::yadisk::PublicDisk;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

const CHECK_PARALLEL: usize = 8;
const UPLOAD_PARALLEL: usize = 4;

fn public_store(http: reqwest::Client) -> Store {
    Store::Public(PublicDisk::new(
        http,
        scam_core::config::project().yandex.public_url.clone(),
    ))
}

// ---------- init ----------

pub struct InitArgs {
    pub id: Option<String>,
    pub name: Option<String>,
    pub minecraft: String,
    pub loader: LoaderKind,
    pub loader_version: Option<String>,
    pub source: String,
    pub force: bool,
}

pub fn init(config: &Path, args: InitArgs) -> Result<()> {
    if config.exists() && !args.force {
        bail!(
            "{} уже есть (--force, чтобы перезаписать)",
            config.display()
        );
    }
    let dir_name = std::env::current_dir()?
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("pack")
        .to_owned();
    let id = args.id.unwrap_or_else(|| slug(&dir_name));
    if !paths::is_valid_id(&id) {
        bail!("не получилось придумать id из «{dir_name}» — укажи --id");
    }
    let text = packfile::template(&InitOptions {
        name: args.name.unwrap_or(dir_name),
        id,
        minecraft: args.minecraft,
        loader: args.loader,
        loader_version: args.loader_version,
        source: args.source,
    });
    std::fs::write(config, text)?;
    ui::ok(format!("Создан {}", config.display()));
    println!("  Проверь группы и exclude, затем: scam-pack status");
    Ok(())
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(64).collect()
}

// ---------- сбор локальной сборки ----------

struct Local {
    pack: Pack,
    scan: Scan,
    files: Vec<FileEntry>,
}

/// Что делать с файлами вне групп.
#[derive(Clone, Copy, PartialEq)]
enum Unmatched {
    /// Только показать (status).
    Report,
    /// Пропустить (publish --allow-unmatched).
    Skip,
    /// Ошибка (publish).
    Fail,
}

fn collect(config: &Path, unmatched: Unmatched) -> Result<Local> {
    let pack = Pack::load(config)?;
    ui::step(format!(
        "{} — {} {} {}, папка {}",
        style(&pack.meta.name).bold(),
        pack.meta.minecraft,
        pack.loader.title(),
        pack.meta.loader_version.as_deref().unwrap_or(""),
        pack.source.display()
    ));
    let scan = scan::scan(&pack)?;
    if !scan.unmatched.is_empty() {
        let n = scan.unmatched.len();
        let note = match unmatched {
            Unmatched::Report => " — publish откажется, пока их не распределить",
            Unmatched::Skip => " — пропускаю",
            Unmatched::Fail => "",
        };
        ui::warn(format!("Файлов вне групп: {n}{note}"));
        ui::list("?", scan.unmatched.iter().cloned(), 15);
        if unmatched == Unmatched::Fail {
            bail!("добавь их в группу или в exclude, либо запусти с --allow-unmatched");
        }
    }
    let total: u64 = scan.files.iter().map(|f| f.size).sum();
    let bar = ui::bytes_bar(total, "Хэши");
    let files = scan::hash(&pack, &scan, &bar)?;
    bar.finish_and_clear();
    Ok(Local { pack, scan, files })
}

fn print_groups(local: &Local) {
    let mut per: BTreeMap<usize, (usize, u64)> = BTreeMap::new();
    for f in &local.scan.files {
        let e = per.entry(f.group).or_default();
        e.0 += 1;
        e.1 += f.size;
    }
    for (i, g) in local.pack.groups.iter().enumerate() {
        let (n, size) = per.get(&i).copied().unwrap_or_default();
        let mode = match g.mode {
            scam_core::model::GroupMode::Sync => "sync".to_string(),
            scam_core::model::GroupMode::Once => format!("once r{}", g.revision),
        };
        println!(
            "    {:<16} {:<8} {:>5} файлов {:>10}",
            g.id,
            style(mode).dim(),
            n,
            ui::size(size)
        );
    }
    if local.scan.excluded > 0 {
        println!(
            "    {}",
            style(format!("исключено файлов: {}", local.scan.excluded)).dim()
        );
    }
}

// ---------- сравнение ----------

#[derive(Default)]
struct Diff {
    added: Vec<String>,
    changed: Vec<String>,
    removed: Vec<String>,
}

impl Diff {
    fn new(old: &[FileEntry], new: &[FileEntry]) -> Self {
        let old_map: HashMap<&str, &FileEntry> = old.iter().map(|f| (f.path.as_str(), f)).collect();
        let new_set: HashSet<&str> = new.iter().map(|f| f.path.as_str()).collect();
        let mut d = Diff::default();
        for f in new {
            match old_map.get(f.path.as_str()) {
                None => d.added.push(f.path.clone()),
                Some(o) if o.sha1 != f.sha1 || o.group != f.group => d.changed.push(f.path.clone()),
                _ => {}
            }
        }
        d.removed = old
            .iter()
            .filter(|f| !new_set.contains(f.path.as_str()))
            .map(|f| f.path.clone())
            .collect();
        d
    }

    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }

    fn print(&self) {
        if self.is_empty() {
            println!("    файлы не изменились");
            return;
        }
        ui::list(&style("+").green().to_string(), self.added.clone(), 20);
        ui::list(&style("~").yellow().to_string(), self.changed.clone(), 20);
        ui::list(&style("-").red().to_string(), self.removed.clone(), 20);
    }
}

/// Опубликованный билд, с которым сравниваем: текущий в целевом канале, иначе самый свежий.
async fn baseline(
    store: &Store,
    entry: Option<&IndexPack>,
    channel: Channel,
) -> Result<Option<(Channel, BuildManifest)>> {
    let Some(entry) = entry else { return Ok(None) };
    let pick = entry
        .channels
        .get(channel)
        .map(|c| (channel, c.build))
        .or_else(|| {
            [Channel::Stable, Channel::Beta]
                .into_iter()
                .filter_map(|ch| entry.channels.get(ch).map(|c| (ch, c.build)))
                .max_by_key(|(_, b)| *b)
        });
    match pick {
        Some((ch, build)) => Ok(Some((ch, store.build(&entry.id, build).await?))),
        None => Ok(None),
    }
}

fn describe(entry: &IndexPack) -> String {
    let fmt = |c: Option<&ChannelInfo>| {
        c.map(|c| format!("{} (build {})", c.version, c.build))
            .unwrap_or_else(|| "—".into())
    };
    format!(
        "релиз {}, бета {}",
        fmt(entry.channels.stable.as_ref()),
        fmt(entry.channels.beta.as_ref())
    )
}

// ---------- status ----------

pub async fn status(http: reqwest::Client, config: &Path, beta: bool) -> Result<()> {
    let local = collect(config, Unmatched::Report)?;
    print_groups(&local);

    let store = public_store(http);
    let index = store.index().await?;
    let entry = index.pack(&local.pack.meta.id);
    let channel = if beta { Channel::Beta } else { Channel::Stable };
    match entry {
        Some(e) => ui::step(format!("На диске: {}", describe(e))),
        None => ui::step("На диске этой сборки ещё нет — будет первая публикация"),
    }
    let Some((base_ch, base)) = baseline(&store, entry, channel).await? else {
        let total: u64 = local.files.iter().map(|f| f.size).sum();
        ui::ok(format!(
            "К загрузке: {} файлов, {}",
            local.files.len(),
            ui::size(total)
        ));
        return Ok(());
    };
    ui::step(format!(
        "Изменения относительно {} {} (build {}):",
        base_ch, base.version, base.build
    ));
    Diff::new(&base.files, &local.files).print();

    let known: HashSet<&str> = base.files.iter().map(|f| f.sha1.as_str()).collect();
    let mut new_objects: HashMap<&str, u64> = HashMap::new();
    for f in &local.files {
        if !known.contains(f.sha1.as_str()) {
            new_objects.insert(&f.sha1, f.size);
        }
    }
    ui::ok(format!(
        "К загрузке не больше {} файлов, {} (что уже лежит на диске, заливаться не будет)",
        new_objects.len(),
        ui::size(new_objects.values().sum())
    ));
    Ok(())
}

// ---------- publish ----------

pub struct PublishArgs {
    pub beta: bool,
    pub version: Option<String>,
    pub message: Option<String>,
    pub changelog: Option<PathBuf>,
    pub force: bool,
    pub allow_unmatched: bool,
}

pub async fn publish(http: reqwest::Client, config: &Path, args: PublishArgs) -> Result<()> {
    // Сначала вход: не считать хэши гигабайтов, чтобы потом упасть на токене.
    let publisher = Publisher::connect(http).await?;
    let local = collect(
        config,
        if args.allow_unmatched {
            Unmatched::Skip
        } else {
            Unmatched::Fail
        },
    )?;
    print_groups(&local);
    let pack = &local.pack;
    let channel = if args.beta {
        Channel::Beta
    } else {
        Channel::Stable
    };

    let changelog = match (&args.message, &args.changelog) {
        (Some(_), Some(_)) => bail!("укажи что-то одно: -m или --changelog"),
        (Some(m), None) => Some(m.clone()),
        (None, Some(p)) => Some(
            std::fs::read_to_string(p)
                .with_context(|| format!("не удалось прочитать {}", p.display()))?,
        ),
        (None, None) => None,
    };

    let store = publisher.store();
    let index = store.index().await?;
    let entry = index.pack(&pack.meta.id);

    // Манифесты текущих каналов: для сравнения и чтобы не проверять уже залитые объекты.
    let mut published: Vec<BuildManifest> = Vec::new();
    if let Some(e) = entry {
        for ch in [Channel::Stable, Channel::Beta] {
            if let Some(c) = e.channels.get(ch)
                && !published.iter().any(|m| m.build == c.build)
            {
                published.push(store.build(&e.id, c.build).await?);
            }
        }
    }
    let base = baseline(&store, entry, channel).await?;

    let groups = scan::manifest_groups(pack, &local.scan);
    let mut manifest = BuildManifest {
        schema: SCHEMA,
        pack: pack.meta.id.clone(),
        name: pack.meta.name.clone(),
        build: 0,
        version: args
            .version
            .clone()
            .or_else(|| base.as_ref().map(|(_, b)| b.version.clone()))
            .unwrap_or_else(|| "1.0.0".into()),
        created: Utc::now(),
        minecraft: pack.meta.minecraft.clone(),
        loader: Loader {
            kind: pack.loader,
            version: pack.meta.loader_version.clone(),
        },
        java: pack.meta.java,
        memory: Memory {
            min: pack.meta.memory_min,
            recommended: pack.meta.memory_recommended,
        },
        changelog,
        groups,
        files: local.files.clone(),
    };

    if let Some((base_ch, base)) = &base {
        ui::step(format!(
            "Изменения относительно {} {} (build {}):",
            base_ch, base.version, base.build
        ));
        Diff::new(&base.files, &manifest.files).print();
        let same = *base_ch == channel
            && base.files == manifest.files
            && base.groups == manifest.groups
            && base.minecraft == manifest.minecraft
            && base.loader == manifest.loader
            && base.java == manifest.java
            && base.memory == manifest.memory
            && args.version.is_none();
        if same && !args.force {
            ui::ok("Изменений нет — публиковать нечего (--force, чтобы всё равно выпустить билд)");
            return Ok(());
        }
        if channel == Channel::Stable
            && let Some(b) = published
                .iter()
                .find(|m| m.files == manifest.files && m.build != base.build)
        {
            ui::warn(format!(
                "Файлы совпадают с билдом {} — возможно, нужен `scam-pack promote {} --build {}`",
                b.build, pack.meta.id, b.build
            ));
        }
    }

    // Объекты: иконка/фон + файлы, которых нет в уже опубликованных билдах.
    let mut objects: HashMap<String, (PathBuf, u64)> = HashMap::new();
    let known: HashSet<&str> = published
        .iter()
        .flat_map(|m| m.files.iter().map(|f| f.sha1.as_str()))
        .collect();
    let paths_by_rel: HashMap<&str, &PathBuf> = local
        .scan
        .files
        .iter()
        .map(|f| (f.rel.as_str(), &f.abs))
        .collect();
    for f in &manifest.files {
        if !known.contains(f.sha1.as_str()) {
            objects.insert(
                f.sha1.clone(),
                (paths_by_rel[f.path.as_str()].clone(), f.size),
            );
        }
    }
    let mut art = |p: &Option<PathBuf>| -> Result<Option<ObjectRef>> {
        let Some(p) = p else { return Ok(None) };
        let (sha1, size) = scam_core::hash::sha1_file(p)?;
        objects.insert(sha1.clone(), (p.clone(), size));
        Ok(Some(ObjectRef { sha1, size }))
    };
    let icon = art(&pack.icon)?;
    let background = art(&pack.background)?;

    let candidates: Vec<(String, PathBuf, u64)> =
        objects.into_iter().map(|(h, (p, s))| (h, p, s)).collect();
    let bar = ui::count_bar(candidates.len() as u64, "Проверка");
    let missing: Vec<(String, PathBuf, u64)> = stream::iter(candidates)
        .map(|(sha1, path, size)| {
            let publisher = &publisher;
            let bar = &bar;
            async move {
                let exists = publisher.object_exists(&sha1, size).await?;
                bar.inc(1);
                anyhow::Ok((!exists).then_some((sha1, path, size)))
            }
        })
        .buffer_unordered(CHECK_PARALLEL)
        .try_filter_map(|x| async move { Ok(x) })
        .try_collect()
        .await?;
    bar.finish_and_clear();

    if !missing.is_empty() {
        publisher.mkdir_p(paths::OBJECTS_DIR).await?;
        let total: u64 = missing.iter().map(|m| m.2).sum();
        ui::step(format!(
            "Загрузка {} файлов, {}",
            missing.len(),
            ui::size(total)
        ));
        let bar = ui::bytes_bar(total, "Загрузка");
        stream::iter(missing)
            .map(|(sha1, path, size)| {
                let publisher = &publisher;
                let bar = &bar;
                async move {
                    publisher.upload_object(&sha1, &path).await?;
                    bar.inc(size);
                    anyhow::Ok(())
                }
            })
            .buffer_unordered(UPLOAD_PARALLEL)
            .try_collect::<()>()
            .await?;
        bar.finish_and_clear();
    } else {
        ui::step("Все файлы уже есть на диске, заливать нечего");
    }

    // Билд → индекс. Индекс пишется последним: до этого игроки видят прошлую версию.
    let existing = publisher.build_numbers(&pack.meta.id).await?;
    let channel_max = entry
        .map(|e| {
            [e.channels.stable.as_ref(), e.channels.beta.as_ref()]
                .into_iter()
                .flatten()
                .map(|c| c.build)
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    manifest.build = existing.last().copied().unwrap_or(0).max(channel_max) + 1;
    scam_core::remote::validate_manifest(&manifest).map_err(anyhow::Error::msg)?;
    publisher
        .put_json(
            &paths::build_file(&manifest.pack, manifest.build),
            &manifest,
            false,
        )
        .await
        .context("билд с таким номером уже есть — возможно, публикация шла параллельно; повтори")?;

    let mut index = store.index().await?;
    let info = ChannelInfo {
        build: manifest.build,
        version: manifest.version.clone(),
    };
    let now = Utc::now();
    match index.pack_mut(&pack.meta.id) {
        Some(e) => {
            e.name = pack.meta.name.clone();
            e.description = pack.meta.description.clone();
            e.minecraft = pack.meta.minecraft.clone();
            e.loader = pack.loader;
            e.icon = icon;
            e.background = background;
            e.channels.set(channel, info);
            e.updated = now;
        }
        None => {
            let mut e = IndexPack {
                id: pack.meta.id.clone(),
                name: pack.meta.name.clone(),
                description: pack.meta.description.clone(),
                minecraft: pack.meta.minecraft.clone(),
                loader: pack.loader,
                icon,
                background,
                channels: Default::default(),
                updated: now,
            };
            e.channels.set(channel, info);
            index.packs.push(e);
        }
    }
    write_index(&publisher, &mut index).await?;

    ui::ok(format!(
        "Опубликовано: {} {} (build {}) → {}",
        style(&manifest.name).bold(),
        manifest.version,
        manifest.build,
        channel
    ));
    Ok(())
}

async fn write_index(publisher: &Publisher, index: &mut Index) -> Result<()> {
    index.schema = SCHEMA;
    publisher.put_json(paths::INDEX_FILE, index, true).await
}

// ---------- promote ----------

pub async fn promote(http: reqwest::Client, pack_id: &str, build: Option<u64>) -> Result<()> {
    let publisher = Publisher::connect(http).await?;
    let store = publisher.store();
    let mut index = store.index().await?;
    let entry = index
        .pack(pack_id)
        .with_context(|| format!("сборки «{pack_id}» нет на диске"))?;
    let target = match build {
        Some(b) => b,
        None => {
            entry
                .channels
                .beta
                .as_ref()
                .context("у сборки нет беты — укажи --build N")?
                .build
        }
    };
    let manifest = store.build(pack_id, target).await?;
    let old = entry.channels.stable.clone();
    if old.as_ref().is_some_and(|o| o.build == target) {
        ui::ok(format!("Билд {target} уже релиз"));
        return Ok(());
    }

    let e = index.pack_mut(pack_id).unwrap();
    e.channels.stable = Some(ChannelInfo {
        build: target,
        version: manifest.version.clone(),
    });
    e.updated = Utc::now();
    write_index(&publisher, &mut index).await?;

    let was = old
        .map(|o| format!("{} (build {})", o.version, o.build))
        .unwrap_or_else(|| "—".into());
    ui::ok(format!(
        "Релиз {pack_id}: {was} → {} (build {target})",
        manifest.version
    ));
    Ok(())
}

// ---------- list ----------

pub async fn list(http: reqwest::Client) -> Result<()> {
    let store = public_store(http);
    let index = store.index().await?;
    if index.packs.is_empty() {
        ui::step("На диске пока нет сборок");
    }
    for p in &index.packs {
        println!(
            "{} {} — {} {}",
            style(&p.id).bold(),
            style(format!("«{}»", p.name)).dim(),
            p.minecraft,
            p.loader.title()
        );
        println!("    {}", describe(p));
        println!(
            "    {}",
            style(format!(
                "обновлено {}",
                p.updated.format("%Y-%m-%d %H:%M UTC")
            ))
            .dim()
        );
    }
    let news = store.news().await?;
    if !news.items.is_empty() {
        ui::step(format!("Новостей: {}", news.items.len()));
    }
    Ok(())
}

// ---------- news ----------

pub async fn news_add(
    http: reqwest::Client,
    title: String,
    text: String,
    pack: Option<String>,
) -> Result<()> {
    let publisher = Publisher::connect(http).await?;
    let store = publisher.store();
    if let Some(p) = &pack
        && store.index().await?.pack(p).is_none()
    {
        bail!("сборки «{p}» нет на диске");
    }
    let mut news = store.news().await?;
    let now = Utc::now();
    let mut id = now.format("%Y%m%d-%H%M%S").to_string();
    while news.items.iter().any(|n| n.id == id) {
        id.push('x');
    }
    news.items.insert(
        0,
        NewsItem {
            id: id.clone(),
            date: now,
            title,
            text,
            pack,
        },
    );
    news.schema = SCHEMA;
    publisher.put_json(paths::NEWS_FILE, &news, true).await?;
    ui::ok(format!("Новость добавлена ({id})"));
    Ok(())
}

pub async fn news_list(http: reqwest::Client) -> Result<()> {
    let news = public_store(http).news().await?;
    if news.items.is_empty() {
        ui::step("Новостей нет");
    }
    for n in &news.items {
        let pack = n
            .pack
            .as_deref()
            .map(|p| format!(" [{p}]"))
            .unwrap_or_default();
        println!(
            "{} {}{} — {}",
            style(&n.id).dim(),
            n.date.format("%Y-%m-%d"),
            pack,
            style(&n.title).bold()
        );
    }
    Ok(())
}

pub async fn news_rm(http: reqwest::Client, id: &str) -> Result<()> {
    let publisher = Publisher::connect(http).await?;
    let mut news = publisher.store().news().await?;
    let before = news.items.len();
    news.items.retain(|n| n.id != id);
    if news.items.len() == before {
        bail!("новости {id} нет");
    }
    publisher.put_json(paths::NEWS_FILE, &news, true).await?;
    ui::ok(format!("Новость {id} удалена"));
    Ok(())
}

// ---------- gc ----------

pub async fn gc(
    http: reqwest::Client,
    keep: Option<usize>,
    permanently: bool,
    yes: bool,
) -> Result<()> {
    let publisher = Publisher::connect(http).await?;
    let store = publisher.store();
    let index = store.index().await?;

    let mut delete_builds: Vec<String> = Vec::new();
    let mut referenced: HashSet<String> = HashSet::new();
    for p in &index.packs {
        referenced.extend(p.icon.iter().chain(&p.background).map(|o| o.sha1.clone()));
        let builds = publisher.build_numbers(&p.id).await?;
        let pinned: HashSet<u64> = [p.channels.stable.as_ref(), p.channels.beta.as_ref()]
            .into_iter()
            .flatten()
            .map(|c| c.build)
            .collect();
        let recent: HashSet<u64> = match keep {
            Some(k) => builds.iter().rev().take(k).copied().collect(),
            None => builds.iter().copied().collect(),
        };
        let bar = ui::count_bar(builds.len() as u64, &p.id);
        for b in &builds {
            if pinned.contains(b) || recent.contains(b) {
                let m = store.build(&p.id, *b).await?;
                referenced.extend(m.files.into_iter().map(|f| f.sha1));
            } else {
                delete_builds.push(paths::build_file(&p.id, *b));
            }
            bar.inc(1);
        }
        bar.finish_and_clear();
    }

    let mut orphans: Vec<(String, u64)> = Vec::new();
    let prefixes = match publisher
        .disk
        .list_dir(&publisher.path(paths::OBJECTS_DIR))
        .await
    {
        Ok(items) => items,
        Err(e) if e.is_not_found() => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    let bar = ui::count_bar(prefixes.len() as u64, "Объекты");
    for dir in prefixes.iter().filter(|d| d.is_dir()) {
        for obj in publisher.disk.list_dir(&dir.path).await? {
            if paths::is_sha1(&obj.name) && !referenced.contains(&obj.name) {
                orphans.push((obj.name.clone(), obj.size.unwrap_or(0)));
            }
        }
        bar.inc(1);
    }
    bar.finish_and_clear();

    let freed: u64 = orphans.iter().map(|o| o.1).sum();
    ui::step(format!(
        "Старых билдов к удалению: {}, неиспользуемых файлов: {} ({})",
        delete_builds.len(),
        orphans.len(),
        ui::size(freed)
    ));
    ui::list("-", delete_builds.iter().cloned(), 10);
    if delete_builds.is_empty() && orphans.is_empty() {
        ui::ok("Чистить нечего");
        return Ok(());
    }
    if !yes {
        ui::warn("Пробный прогон. Чтобы удалить, добавь --yes");
        return Ok(());
    }

    let targets: Vec<String> = delete_builds
        .into_iter()
        .chain(orphans.into_iter().map(|(h, _)| paths::object_file(&h)))
        .collect();
    let bar = ui::count_bar(targets.len() as u64, "Удаление");
    stream::iter(targets)
        .map(|rel| {
            let publisher = &publisher;
            let bar = &bar;
            async move {
                publisher.delete(&rel, permanently).await?;
                bar.inc(1);
                anyhow::Ok(())
            }
        })
        .buffer_unordered(CHECK_PARALLEL)
        .try_collect::<()>()
        .await?;
    bar.finish_and_clear();
    let where_to = if permanently {
        "навсегда"
    } else {
        "в корзину Диска"
    };
    ui::ok(format!(
        "Удалено {where_to}, освобождено {}",
        ui::size(freed)
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fe(path: &str, sha1: &str) -> FileEntry {
        FileEntry {
            path: path.into(),
            sha1: sha1.into(),
            size: 1,
            group: "g".into(),
            mod_ids: vec![],
        }
    }

    #[test]
    fn diff() {
        let old = [fe("a", "1"), fe("b", "2"), fe("c", "3")];
        let new = [fe("a", "1"), fe("b", "9"), fe("d", "4")];
        let d = Diff::new(&old, &new);
        assert_eq!(d.added, vec!["d"]);
        assert_eq!(d.changed, vec!["b"]);
        assert_eq!(d.removed, vec!["c"]);
        assert!(Diff::new(&old, &old).is_empty());
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("My Pack 2"), "my-pack-2");
        assert_eq!(slug("Техно Pack"), "pack");
        assert_eq!(slug("__x__"), "x");
    }
}
