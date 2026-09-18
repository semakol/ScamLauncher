//! Загрузчики модов: Fabric, Quilt (профиль с meta-сервера) и Forge, NeoForge (установщик + processors).
//!
//! Результат — JSON версии загрузчика в `versions/<id>/<id>.json` с `inheritsFrom` на ванильную версию.

use crate::dirs::GameDirs;
use crate::net::{FileSpec, Net, Progress, ProgressSink};
use crate::rules::Env;
use crate::version::{Library, Maven, VersionJson};
use anyhow::{Context, Result, bail};
use scam_core::model::LoaderKind;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

pub struct LoaderCtx<'a> {
    pub net: &'a Net,
    pub dirs: &'a GameDirs,
    pub env: &'a Env,
    pub java: &'a Path,
    pub verify: bool,
    pub progress: &'a dyn ProgressSink,
}

/// Ставит загрузчик и возвращает id его версии.
pub async fn ensure(
    ctx: &LoaderCtx<'_>,
    mc: &str,
    kind: LoaderKind,
    version: &str,
) -> Result<String> {
    match kind {
        LoaderKind::Vanilla => Ok(mc.to_owned()),
        LoaderKind::Fabric => {
            meta_profile(
                ctx,
                "Fabric",
                "https://meta.fabricmc.net/v2",
                "fabric-loader",
                mc,
                version,
            )
            .await
        }
        LoaderKind::Quilt => {
            meta_profile(
                ctx,
                "Quilt",
                "https://meta.quiltmc.org/v3",
                "quilt-loader",
                mc,
                version,
            )
            .await
        }
        LoaderKind::Forge | LoaderKind::NeoForge => forge_like(ctx, mc, kind, version).await,
    }
}

// ---------- Fabric / Quilt ----------

async fn meta_profile(
    ctx: &LoaderCtx<'_>,
    title: &str,
    base: &str,
    prefix: &str,
    mc: &str,
    version: &str,
) -> Result<String> {
    let guess = format!("{prefix}-{version}-{mc}");
    if !ctx.verify && ctx.dirs.version_json(&guess).is_file() {
        return Ok(guess);
    }
    (ctx.progress)(Progress::Stage(format!("Установка {title} {version}")));
    let url = format!("{base}/versions/loader/{mc}/{version}/profile/json");
    let bytes = ctx
        .net
        .get_bytes(&url)
        .await
        .with_context(|| format!("{title} {version} для Minecraft {mc} не найден"))?;
    let json: VersionJson =
        serde_json::from_slice(&bytes).with_context(|| format!("странный ответ {title}: {url}"))?;
    write_version(ctx.dirs, &json.id, &bytes)?;
    Ok(json.id)
}

fn write_version(dirs: &GameDirs, id: &str, bytes: &[u8]) -> Result<()> {
    if !scam_core::paths::is_safe_rel_path(id) || id.contains('/') {
        bail!("странный id версии: {id}");
    }
    let path = dirs.version_json(id);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, bytes)?;
    Ok(())
}

// ---------- Forge / NeoForge ----------

#[derive(Deserialize)]
struct InstallProfile {
    #[serde(default)]
    json: Option<String>,
    #[serde(default)]
    data: HashMap<String, DataEntry>,
    #[serde(default)]
    processors: Vec<Processor>,
    #[serde(default)]
    libraries: Vec<Library>,
}

#[derive(Deserialize)]
struct DataEntry {
    client: String,
}

#[derive(Deserialize)]
struct Processor {
    jar: String,
    #[serde(default)]
    classpath: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    outputs: HashMap<String, String>,
    #[serde(default)]
    sides: Option<Vec<String>>,
}

/// Координаты установщика. Версию можно писать и `47.4.0`, и `1.20.1-47.4.0`.
fn installer(kind: LoaderKind, mc: &str, version: &str) -> (String, Maven) {
    let full = |v: &str| {
        if v.starts_with(&format!("{mc}-")) {
            v.to_owned()
        } else {
            format!("{mc}-{v}")
        }
    };
    let (base, group, artifact, ver) = match kind {
        LoaderKind::NeoForge if mc == "1.20.1" => (
            "https://maven.neoforged.net/releases/",
            "net.neoforged",
            "forge",
            full(version),
        ),
        LoaderKind::NeoForge => (
            "https://maven.neoforged.net/releases/",
            "net.neoforged",
            "neoforge",
            version.to_owned(),
        ),
        _ => (
            "https://maven.minecraftforge.net/",
            "net.minecraftforge",
            "forge",
            full(version),
        ),
    };
    (
        base.to_owned(),
        Maven {
            group: group.into(),
            artifact: artifact.into(),
            version: ver,
            classifier: Some("installer".into()),
            ext: "jar".into(),
        },
    )
}

async fn forge_like(
    ctx: &LoaderCtx<'_>,
    mc: &str,
    kind: LoaderKind,
    version: &str,
) -> Result<String> {
    let title = kind.title();
    let (base, coords) = installer(kind, mc, version);
    let installer_path = ctx.dirs.libraries().join(coords.path());
    let url = format!("{base}{}", coords.path());

    // Установщик без sha1 на сервере: битый архив — перекачиваем.
    let mut zip = None;
    for attempt in 0..2 {
        if attempt > 0 || !installer_path.is_file() {
            (ctx.progress)(Progress::Stage(format!(
                "Загрузка установщика {title} {version}"
            )));
            let _ = std::fs::remove_file(&installer_path);
            ctx.net
                .ensure(&FileSpec::new(&url, &installer_path), false)
                .await
                .with_context(|| format!("{title} {version} для Minecraft {mc} не найден"))?;
        }
        match std::fs::File::open(&installer_path).map(zip::ZipArchive::new) {
            Ok(Ok(z)) => {
                zip = Some(z);
                break;
            }
            _ => continue,
        }
    }
    let mut zip =
        zip.with_context(|| format!("установщик {title} повреждён: {}", installer_path.display()))?;

    let profile: InstallProfile =
        serde_json::from_slice(&read_entry(&mut zip, "install_profile.json")?)
            .context("не удалось прочитать install_profile.json")?;
    let version_path = profile
        .json
        .as_deref()
        .unwrap_or("/version.json")
        .trim_start_matches('/')
        .to_owned();
    let version_bytes = read_entry(&mut zip, &version_path)?;
    let version_json: VersionJson = serde_json::from_slice(&version_bytes)
        .context("не удалось прочитать version.json установщика")?;
    let id = version_json.id.clone();

    let (installer_sha1, _) = scam_core::hash::sha1_file(&installer_path)?;
    let marker = ctx.dirs.versions().join(&id).join(".scam-installed");
    if !ctx.verify
        && ctx.dirs.version_json(&id).is_file()
        && std::fs::read_to_string(&marker).is_ok_and(|m| m.trim() == installer_sha1)
    {
        return Ok(id);
    }

    (ctx.progress)(Progress::Stage(format!("Установка {title} {version}")));
    write_version(ctx.dirs, &id, &version_bytes)?;

    // Библиотеки установщика и версии: скачать или достать из maven/ внутри установщика.
    let libraries = ctx.dirs.libraries();
    let mut specs = Vec::new();
    for lib in profile.libraries.iter().chain(&version_json.libraries) {
        let Some(m) = lib.maven() else { continue };
        let artifact = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref());
        let rel = artifact
            .and_then(|a| a.path.clone())
            .unwrap_or_else(|| m.path());
        if !scam_core::paths::is_safe_rel_path(&rel) {
            bail!("небезопасный путь библиотеки {rel}");
        }
        let path = libraries.join(&rel);
        let embedded = format!("maven/{rel}");
        match artifact {
            Some(a) if !a.url.is_empty() => specs.push(
                FileSpec::new(&a.url, &path)
                    .sha1(a.sha1.as_deref())
                    .size(a.size),
            ),
            _ if zip.by_name(&embedded).is_ok() => extract_entry(&mut zip, &embedded, &path)?,
            // Нет ни ссылки, ни файла в установщике — его создаст один из processors.
            _ => {}
        }
    }
    ctx.net.ensure_all(specs, ctx.verify, ctx.progress).await?;

    // Данные для processors.
    let tmp = ctx.dirs.root.join("tmp").join(&id);
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(&tmp)?;
    let path_str = |p: &Path| p.to_string_lossy().into_owned();
    let mut data: HashMap<String, String> = HashMap::new();
    for (key, entry) in &profile.data {
        let v = &entry.client;
        let value = if let Some(coords) = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            path_str(&maven_path(&libraries, coords)?)
        } else if let Some(lit) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
            lit.to_owned()
        } else if let Some(inner) = v.strip_prefix('/') {
            let out = tmp.join(inner.replace('/', "_"));
            extract_entry(&mut zip, inner, &out)?;
            path_str(&out)
        } else {
            v.clone()
        };
        data.insert(key.clone(), value);
    }
    data.insert("SIDE".into(), "client".into());
    data.insert("MINECRAFT_JAR".into(), path_str(&ctx.dirs.client_jar(mc)));
    data.insert("MINECRAFT_VERSION".into(), mc.to_owned());
    data.insert("ROOT".into(), path_str(&ctx.dirs.root));
    data.insert("INSTALLER".into(), path_str(&installer_path));
    data.insert("LIBRARY_DIR".into(), path_str(&libraries));
    drop(zip);

    let client: Vec<&Processor> = profile
        .processors
        .iter()
        .filter(|p| {
            p.sides
                .as_ref()
                .is_none_or(|s| s.iter().any(|x| x == "client"))
        })
        .collect();
    for (i, proc) in client.iter().enumerate() {
        (ctx.progress)(Progress::Stage(format!(
            "Установка {title} {version}: шаг {} из {}",
            i + 1,
            client.len()
        )));
        run_processor(ctx, proc, &data, &libraries)
            .await
            .with_context(|| format!("шаг установки {title} {} ({}) не удался", i + 1, proc.jar))?;
    }

    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::write(&marker, &installer_sha1)?;
    Ok(id)
}

fn maven_path(libraries: &Path, coords: &str) -> Result<PathBuf> {
    let m = Maven::parse(coords).with_context(|| format!("странные координаты {coords}"))?;
    let rel = m.path();
    if !scam_core::paths::is_safe_rel_path(&rel) {
        bail!("небезопасный путь {rel}");
    }
    Ok(libraries.join(rel))
}

/// `[coords]` → путь библиотеки; `{KEY}` → значение из data.
fn resolve_arg(arg: &str, data: &HashMap<String, String>, libraries: &Path) -> Result<String> {
    if let Some(coords) = arg.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return Ok(maven_path(libraries, coords)?
            .to_string_lossy()
            .into_owned());
    }
    let mut out = String::new();
    let mut rest = arg;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                match data.get(key) {
                    Some(v) => out.push_str(v),
                    None => bail!("processors: неизвестный параметр {{{key}}}"),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

async fn run_processor(
    ctx: &LoaderCtx<'_>,
    proc: &Processor,
    data: &HashMap<String, String>,
    libraries: &Path,
) -> Result<()> {
    // Все выходы на месте и совпадают — шаг уже сделан.
    let outputs: Vec<(PathBuf, String)> = proc
        .outputs
        .iter()
        .map(|(k, v)| {
            Ok((
                PathBuf::from(resolve_arg(k, data, libraries)?),
                resolve_arg(v, data, libraries)?,
            ))
        })
        .collect::<Result<_>>()?;
    if !outputs.is_empty() && !ctx.verify && outputs_ok(&outputs) {
        return Ok(());
    }

    let jar = maven_path(libraries, &proc.jar)?;
    let main_class = main_class(&jar)?;
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut cp = vec![jar.to_string_lossy().into_owned()];
    for c in &proc.classpath {
        cp.push(maven_path(libraries, c)?.to_string_lossy().into_owned());
    }
    let args: Vec<String> = proc
        .args
        .iter()
        .map(|a| resolve_arg(a, data, libraries))
        .collect::<Result<_>>()?;

    let mut cmd = tokio::process::Command::new(ctx.java);
    cmd.arg("-cp")
        .arg(cp.join(sep))
        .arg(&main_class)
        .args(&args);
    cmd.current_dir(&ctx.dirs.root);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = cmd.output().await.context("не удалось запустить java")?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr).into_owned()
            + &String::from_utf8_lossy(&out.stdout);
        let tail: Vec<&str> = text.lines().rev().take(15).collect();
        bail!(
            "{main_class} завершился с кодом {:?}:\n{}",
            out.status.code(),
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        );
    }
    if !outputs.is_empty() && !outputs_ok(&outputs) {
        bail!("{main_class}: результат не совпал с ожидаемым (sha1)");
    }
    Ok(())
}

fn outputs_ok(outputs: &[(PathBuf, String)]) -> bool {
    outputs.iter().all(|(path, sha1)| {
        scam_core::hash::sha1_file(path).is_ok_and(|(actual, _)| actual.eq_ignore_ascii_case(sha1))
    })
}

fn main_class(jar: &Path) -> Result<String> {
    let file = std::fs::File::open(jar).with_context(|| format!("нет {}", jar.display()))?;
    let mut zip = zip::ZipArchive::new(file)?;
    let manifest = String::from_utf8(read_entry(&mut zip, "META-INF/MANIFEST.MF")?)?;
    manifest
        .lines()
        .find_map(|l| l.strip_prefix("Main-Class:"))
        .map(|s| s.trim().to_owned())
        .with_context(|| format!("в {} нет Main-Class", jar.display()))
}

fn read_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<Vec<u8>> {
    let mut entry = zip
        .by_name(name)
        .with_context(|| format!("в архиве нет {name}"))?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

fn extract_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str, out: &Path) -> Result<()> {
    let bytes = read_entry(zip, name)?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_coords() {
        let (base, m) = installer(LoaderKind::Forge, "1.20.1", "47.4.0");
        assert_eq!(
            format!("{base}{}", m.path()),
            "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.4.0/forge-1.20.1-47.4.0-installer.jar"
        );
        let (_, m) = installer(LoaderKind::Forge, "1.16.5", "1.16.5-36.2.42");
        assert_eq!(m.version, "1.16.5-36.2.42");
        let (base, m) = installer(LoaderKind::NeoForge, "1.21.1", "21.1.250");
        assert_eq!(
            format!("{base}{}", m.path()),
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.250/neoforge-21.1.250-installer.jar"
        );
        let (_, m) = installer(LoaderKind::NeoForge, "1.20.1", "47.1.106");
        assert_eq!(
            m.path(),
            "net/neoforged/forge/1.20.1-47.1.106/forge-1.20.1-47.1.106-installer.jar"
        );
    }

    #[test]
    fn processor_args() {
        let libs = Path::new("/libs");
        let mut data = HashMap::new();
        data.insert("SIDE".to_string(), "client".to_string());
        data.insert("MC_SLIM".to_string(), "/libs/slim.jar".to_string());
        assert_eq!(resolve_arg("{SIDE}", &data, libs).unwrap(), "client");
        assert_eq!(
            resolve_arg("--x={MC_SLIM}", &data, libs).unwrap(),
            "--x=/libs/slim.jar"
        );
        assert_eq!(
            resolve_arg(
                "[net.minecraftforge:forge:1.20.1-47.4.0:client]",
                &data,
                libs
            )
            .unwrap(),
            "/libs/net/minecraftforge/forge/1.20.1-47.4.0/forge-1.20.1-47.4.0-client.jar"
        );
        assert_eq!(resolve_arg("--plain", &data, libs).unwrap(), "--plain");
        assert!(resolve_arg("{NOPE}", &data, libs).is_err());
    }
}
