//! Обход папки сборки: раскладка файлов по группам, корни групп, хэши.

use crate::packfile::Pack;
use anyhow::{Context, Result, bail};
use indicatif::ProgressBar;
use scam_core::model::{FileEntry, Group, GroupMode};
use scam_core::paths::is_safe_rel_path;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub rel: String,
    pub abs: PathBuf,
    pub size: u64,
    /// Индекс в `pack.groups`.
    pub group: usize,
    /// Корень, который дала маска (для once-групп).
    pub root: String,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub files: Vec<ScannedFile>,
    pub unmatched: Vec<String>,
    pub excluded: usize,
}

pub fn scan(pack: &Pack) -> Result<Scan> {
    let mut out = Scan::default();
    let source = &pack.source;
    let walker = WalkDir::new(source)
        .follow_links(true)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            match rel_path(source, e.path()) {
                Some(rel) => !pack.exclude.covers_dir(&rel),
                None => true,
            }
        });

    for entry in walker {
        let entry = entry.with_context(|| format!("не удалось обойти {}", source.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(rel) = rel_path(source, entry.path()) else {
            bail!("имя файла не в UTF-8: {}", entry.path().display());
        };
        if !is_safe_rel_path(&rel) {
            bail!("недопустимое имя файла: {rel}");
        }
        if pack.exclude.is_match(&rel) {
            out.excluded += 1;
            continue;
        }
        let matched = pack
            .groups
            .iter()
            .enumerate()
            .find_map(|(i, g)| g.include.first_match(&rel).map(|p| (i, p)));
        let Some((group, pattern)) = matched else {
            out.unmatched.push(rel);
            continue;
        };
        let root = pattern
            .static_root()
            .unwrap_or_else(|| rel.split('/').next().unwrap_or(&rel).to_owned());
        out.files.push(ScannedFile {
            size: entry.metadata()?.len(),
            abs: entry.into_path(),
            rel,
            group,
            root,
        });
    }
    Ok(out)
}

fn rel_path(base: &std::path::Path, path: &std::path::Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("/"))
}

/// Группы для манифеста: у once-групп — корни (явные или выведенные из масок).
pub fn manifest_groups(pack: &Pack, scan: &Scan) -> Vec<Group> {
    pack.groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let roots = match g.mode {
                GroupMode::Sync => Vec::new(),
                GroupMode::Once if !g.roots.is_empty() => g.roots.clone(),
                GroupMode::Once => collapse_roots(
                    scan.files
                        .iter()
                        .filter(|f| f.group == i)
                        .map(|f| f.root.clone())
                        .collect(),
                ),
            };
            Group {
                id: g.id.clone(),
                name: g.name.clone(),
                mode: g.mode,
                revision: g.revision,
                roots,
                prune: g.prune.clone(),
                optional: g.optional,
                enabled_by_default: g.enabled_by_default,
                description: g.description.clone(),
            }
        })
        .collect()
}

/// Убирает корни, вложенные в другие: {config, config/a.toml} → {config}.
fn collapse_roots(roots: BTreeSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in roots {
        if !out.iter().any(|o| r.starts_with(&format!("{o}/"))) {
            out.push(r);
        }
    }
    out
}

/// Считает sha1 (параллельно) и достаёт mod id из jar.
pub fn hash(pack: &Pack, scan: &Scan, progress: &ProgressBar) -> Result<Vec<FileEntry>> {
    let files = &scan.files;
    let next = AtomicUsize::new(0);
    let results: Mutex<BTreeMap<usize, Result<FileEntry>>> = Mutex::new(BTreeMap::new());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));

    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(f) = files.get(i) else { break };
                    let entry = scam_core::hash::sha1_file(&f.abs)
                        .with_context(|| format!("не удалось прочитать {}", f.abs.display()))
                        .map(|(sha1, size)| FileEntry {
                            path: f.rel.clone(),
                            sha1,
                            size,
                            group: pack.groups[f.group].id.clone(),
                            mod_ids: if f.rel.to_ascii_lowercase().ends_with(".jar") {
                                scam_core::modid::mod_ids_from_jar(&f.abs)
                            } else {
                                Vec::new()
                            },
                        });
                    results.lock().unwrap().insert(i, entry);
                    progress.inc(f.size);
                }
            });
        }
    });

    let mut entries = results
        .into_inner()
        .unwrap()
        .into_values()
        .collect::<Result<Vec<_>>>()?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packfile::{FILE_NAME, Pack};

    fn setup(files: &[&str], toml_groups: &str) -> (tempfile::TempDir, Pack) {
        let dir = tempfile::tempdir().unwrap();
        for f in files {
            let p = dir.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, f.as_bytes()).unwrap();
        }
        let toml_text = format!(
            "[pack]\nid=\"p\"\nname=\"P\"\nminecraft=\"1.20.1\"\nloader=\"vanilla\"\nexclude=[\"saves/**\", \"*.log\"]\n{toml_groups}"
        );
        std::fs::write(dir.path().join(FILE_NAME), toml_text).unwrap();
        let pack = Pack::load(&dir.path().join(FILE_NAME)).unwrap();
        (dir, pack)
    }

    const GROUPS: &str = r#"
[[group]]
id = "mods"
mode = "sync"
include = ["mods/*.jar"]
prune = ["mods/*.jar"]

[[group]]
id = "controls"
mode = "once"
include = ['re:^options(of)?\.txt$']

[[group]]
id = "configs"
mode = "once"
include = ["config/**", "defaultconfigs/**", "extra/one.toml"]

[[group]]
id = "shaders"
mode = "once"
revision = 3
roots = ["shaderpacks"]
include = ["shaderpacks/*.zip"]
"#;

    #[test]
    fn groups_excludes_and_roots() {
        let (_dir, pack) = setup(
            &[
                "mods/a.jar",
                "mods/b.jar",
                "mods/readme.txt",
                "options.txt",
                "optionsof.txt",
                "config/a/b.toml",
                "defaultconfigs/x.toml",
                "extra/one.toml",
                "shaderpacks/BSL.zip",
                "saves/world/level.dat",
                "latest.log",
                ".DS_Store",
                "mods-clients/mine.jar",
            ],
            GROUPS,
        );
        let scan = scan(&pack).unwrap();
        let by_group = |g: &str| -> Vec<&str> {
            let i = pack.groups.iter().position(|x| x.id == g).unwrap();
            scan.files
                .iter()
                .filter(|f| f.group == i)
                .map(|f| f.rel.as_str())
                .collect()
        };
        assert_eq!(by_group("mods"), vec!["mods/a.jar", "mods/b.jar"]);
        assert_eq!(by_group("controls"), vec!["options.txt", "optionsof.txt"]);
        assert_eq!(scan.unmatched, vec!["mods/readme.txt"]);
        // saves/ и mods-clients/ отсекаются целыми папками (не считаются),
        // latest.log, .DS_Store и сам pack.toml — пофайлово.
        assert_eq!(scan.excluded, 3);

        let groups = manifest_groups(&pack, &scan);
        let roots = |id: &str| groups.iter().find(|g| g.id == id).unwrap().roots.clone();
        assert!(roots("mods").is_empty());
        assert_eq!(roots("controls"), vec!["options.txt", "optionsof.txt"]);
        assert_eq!(
            roots("configs"),
            vec!["config", "defaultconfigs", "extra/one.toml"]
        );
        assert_eq!(roots("shaders"), vec!["shaderpacks"]);
        assert_eq!(
            groups.iter().find(|g| g.id == "shaders").unwrap().revision,
            3
        );
    }

    #[test]
    fn hashing() {
        let (_dir, pack) = setup(&["mods/a.jar", "options.txt"], GROUPS);
        let scan = scan(&pack).unwrap();
        let files = hash(&pack, &scan, &ProgressBar::hidden()).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "mods/a.jar");
        assert_eq!(files[0].group, "mods");
        assert_eq!(files[0].sha1, scam_core::hash::sha1_bytes(b"mods/a.jar"));
        assert_eq!(files[1].group, "controls");
    }

    #[test]
    fn collapse() {
        let set = ["config", "config/a.toml", "configx", "a/b", "a/b/c"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(collapse_roots(set), vec!["a/b", "config", "configx"]);
    }
}
