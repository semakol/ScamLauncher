//! Java от Mojang: те же сборки, что ставит официальный лаунчер.

use crate::dirs::GameDirs;
use crate::net::{FileSpec, Net, Progress, ProgressSink};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const ALL_JSON: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

#[derive(Deserialize)]
struct Entry {
    manifest: ManifestRef,
    version: VersionName,
}

#[derive(Deserialize)]
struct ManifestRef {
    sha1: String,
    size: u64,
    url: String,
}

#[derive(Deserialize)]
struct VersionName {
    name: String,
}

#[derive(Deserialize)]
struct RuntimeManifest {
    files: HashMap<String, RuntimeFile>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum RuntimeFile {
    File {
        #[serde(default)]
        executable: bool,
        downloads: RuntimeDownloads,
    },
    Directory,
    Link {
        target: String,
    },
}

#[derive(Deserialize)]
struct RuntimeDownloads {
    raw: Raw,
}

#[derive(Deserialize)]
struct Raw {
    sha1: String,
    size: u64,
    url: String,
}

/// Ключ платформы в `all.json`.
pub fn platform(os: &str, arch: &str) -> Result<&'static str> {
    Ok(match (os, arch) {
        ("osx", "aarch64") => "mac-os-arm64",
        ("osx", _) => "mac-os",
        ("windows", "x86_64") => "windows-x64",
        ("windows", "aarch64") => "windows-arm64",
        ("windows", "x86") => "windows-x86",
        ("linux", "x86_64") => "linux",
        ("linux", "x86") => "linux-i386",
        _ => bail!("Java от Mojang не выпускается для {os} {arch}"),
    })
}

/// Мажорная версия из имени сборки: `17.0.15` → 17, `8u51` → 8, `16.0.1.9.1_3` → 16.
fn major_of(name: &str) -> Option<u32> {
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

pub struct JavaRequest<'a> {
    /// Компонент из JSON версии (`java-runtime-gamma`).
    pub component: Option<&'a str>,
    pub major: u32,
}

/// Ставит (если нужно) Java и возвращает путь к исполняемому файлу.
pub async fn ensure(
    net: &Net,
    dirs: &GameDirs,
    platform: &str,
    req: JavaRequest<'_>,
    verify: bool,
    progress: &dyn ProgressSink,
) -> Result<PathBuf> {
    let all_path = dirs.runtime().join("all.json");
    let all: HashMap<String, HashMap<String, Vec<Entry>>> = net
        .cached_json(&FileSpec::new(ALL_JSON, &all_path))
        .await
        .context("не удалось получить список Java от Mojang")?;
    let components = all
        .get(platform)
        .with_context(|| format!("нет Java для платформы {platform}"))?;

    // Сначала точный компонент, иначе любой с нужной мажорной версией.
    let (component, entry) = req
        .component
        .and_then(|c| {
            components
                .get(c)
                .and_then(|v| v.first())
                .map(|e| (c.to_owned(), e))
        })
        .or_else(|| {
            let mut found: Vec<(&String, &Entry)> = components
                .iter()
                .filter(|(k, _)| !k.contains("snapshot"))
                .filter_map(|(k, v)| v.first().map(|e| (k, e)))
                .filter(|(_, e)| major_of(&e.version.name) == Some(req.major))
                .collect();
            found.sort_by(|a, b| a.0.cmp(b.0));
            found.first().map(|(k, e)| ((*k).clone(), *e))
        })
        .with_context(|| format!("у Mojang нет Java {} для {platform}", req.major))?;

    let home = dirs.runtime().join(format!("{component}-{platform}"));
    let marker = home.join(".scam-installed");
    let installed = std::fs::read_to_string(&marker).unwrap_or_default();
    if installed.trim() == entry.manifest.sha1
        && !verify
        && let Some(java) = find_java(&home)
    {
        return Ok(java);
    }

    progress(Progress::Stage(format!(
        "Загрузка Java {}",
        entry.version.name
    )));
    let manifest: RuntimeManifest = net
        .cached_json(
            &FileSpec::new(
                &entry.manifest.url,
                dirs.runtime().join(format!("{component}-{platform}.json")),
            )
            .sha1(Some(&entry.manifest.sha1))
            .size(Some(entry.manifest.size)),
        )
        .await?;

    let mut specs = Vec::new();
    let mut links = Vec::new();
    for (rel, file) in &manifest.files {
        if !scam_core::paths::is_safe_rel_path(rel) {
            bail!("странный путь в сборке Java: {rel}");
        }
        let path = home.join(rel);
        match file {
            RuntimeFile::Directory => std::fs::create_dir_all(&path)?,
            RuntimeFile::File {
                executable,
                downloads,
            } => specs.push(FileSpec {
                url: downloads.raw.url.clone(),
                path,
                sha1: Some(downloads.raw.sha1.clone()),
                size: Some(downloads.raw.size),
                executable: *executable,
            }),
            RuntimeFile::Link { target } => links.push((path, target.clone())),
        }
    }
    net.ensure_all(specs, verify, progress).await?;
    make_links(&links)?;

    let java = find_java(&home).with_context(|| format!("в {} не нашлось java", home.display()))?;
    std::fs::write(&marker, &entry.manifest.sha1)?;
    Ok(java)
}

#[cfg(unix)]
fn make_links(links: &[(PathBuf, String)]) -> Result<()> {
    for (path, target) in links {
        if std::fs::symlink_metadata(path).is_ok() {
            std::fs::remove_file(path)?;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::os::unix::fs::symlink(target, path)
            .with_context(|| format!("не удалось создать ссылку {}", path.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_links(_links: &[(PathBuf, String)]) -> Result<()> {
    Ok(())
}

fn find_java(home: &Path) -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(windows) {
        &["bin/javaw.exe", "bin/java.exe"]
    } else {
        &["bin/java", "jre.bundle/Contents/Home/bin/java"]
    };
    candidates
        .iter()
        .map(|c| home.join(c))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn majors() {
        assert_eq!(major_of("17.0.15"), Some(17));
        assert_eq!(major_of("8u51-cacert462b08"), Some(8));
        assert_eq!(major_of("16.0.1.9.1_3"), Some(16));
        assert_eq!(major_of("x"), None);
    }

    #[test]
    fn platforms() {
        assert_eq!(platform("osx", "aarch64").unwrap(), "mac-os-arm64");
        assert_eq!(platform("osx", "x86_64").unwrap(), "mac-os");
        assert_eq!(platform("windows", "x86_64").unwrap(), "windows-x64");
        assert_eq!(platform("linux", "x86_64").unwrap(), "linux");
        assert!(platform("linux", "aarch64").is_err());
    }
}
