//! Подготовка версии к запуску: JSON, библиотеки, client.jar, ассеты, натив, Java.

use crate::dirs::GameDirs;
use crate::java::{self, JavaRequest};
use crate::loader;
use crate::net::{FileSpec, Net, Progress, ProgressSink};
use crate::rules::{self, Env};
use crate::version::{Library, Maven, VersionJson};
use anyhow::{Context, Result, bail};
use scam_core::model::LoaderKind;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

const VERSION_MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const LIBRARIES_BASE: &str = "https://libraries.minecraft.net/";
const ASSETS_BASE: &str = "https://resources.download.minecraft.net";

#[derive(Deserialize)]
struct Manifest {
    versions: Vec<ManifestVersion>,
}

#[derive(Deserialize)]
struct ManifestVersion {
    id: String,
    url: String,
    sha1: String,
}

#[derive(Deserialize)]
struct AssetIndex {
    objects: HashMap<String, AssetObject>,
}

#[derive(Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Пересчитать хэши всех файлов («Проверить и починить»).
    pub verify: bool,
    /// Мажорная версия Java вместо той, что указана у версии.
    pub java_major: Option<u32>,
}

/// Всё, что нужно для запуска.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub version: VersionJson,
    pub env: Env,
    pub java: PathBuf,
    pub classpath: Vec<PathBuf>,
    pub natives_dir: PathBuf,
}

/// Что запускать: версия Minecraft и загрузчик.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    pub minecraft: &'a str,
    pub loader: LoaderKind,
    /// Версия загрузчика (для vanilla не нужна).
    pub loader_version: Option<&'a str>,
}

impl<'a> Target<'a> {
    pub fn vanilla(minecraft: &'a str) -> Self {
        Self {
            minecraft,
            loader: LoaderKind::Vanilla,
            loader_version: None,
        }
    }
}

pub async fn prepare(
    net: &Net,
    dirs: &GameDirs,
    target: &Target<'_>,
    opts: &Options,
    progress: &dyn ProgressSink,
) -> Result<Prepared> {
    let mc = target.minecraft;
    progress(Progress::Stage(format!("Проверка версии {mc}")));
    let manifest = mojang_manifest(net, dirs).await;
    let vanilla = load_version(net, dirs, manifest.as_ref().ok(), mc)
        .await
        .with_context(|| match &manifest {
            Err(e) => {
                format!("не удалось получить версию {mc} (список версий Mojang недоступен: {e:#})")
            }
            Ok(_) => format!("не удалось получить версию {mc}"),
        })?;

    // Старые версии без arm64-натив на Mac с M-чипом идут через Rosetta с x64-Java.
    let host = rules::host_arch();
    let arch =
        if rules::host_os() == "osx" && host == "aarch64" && !vanilla.has_macos_arm64_natives() {
            "x86_64"
        } else {
            host
        };
    let env = Env::with_arch(arch);

    let java_major = opts
        .java_major
        .or(vanilla.java_version.as_ref().map(|j| j.major_version))
        .unwrap_or(8);
    let component = match opts.java_major {
        Some(_) => None,
        None => vanilla
            .java_version
            .as_ref()
            .and_then(|j| j.component.as_deref()),
    };
    let java = java::ensure(
        net,
        dirs,
        java::platform(env.os, arch)?,
        JavaRequest {
            component,
            major: java_major,
        },
        opts.verify,
        progress,
    )
    .await?;

    // client.jar нужен раньше остального: из него Forge собирает свою версию игры.
    let client = vanilla
        .downloads
        .as_ref()
        .and_then(|d| d.client.clone())
        .context("в JSON версии нет client.jar")?;
    let vanilla_jar = dirs.client_jar(mc);
    net.ensure(
        &FileSpec::new(&client.url, &vanilla_jar)
            .sha1(client.sha1.as_deref())
            .size(client.size),
        opts.verify,
    )
    .await
    .context("не удалось скачать client.jar")?;

    let version = match (target.loader, target.loader_version) {
        (LoaderKind::Vanilla, _) => vanilla,
        (kind, Some(loader_version)) => {
            let ctx = loader::LoaderCtx {
                net,
                dirs,
                env: &env,
                java: &java,
                verify: opts.verify,
                progress,
            };
            let id = loader::ensure(&ctx, mc, kind, loader_version).await?;
            load_version(net, dirs, manifest.as_ref().ok(), &id).await?
        }
        (kind, None) => bail!("не указана версия {}", kind.title()),
    };

    progress(Progress::Stage("Проверка файлов игры".into()));
    let mut specs = Vec::new();
    let mut classpath = Vec::new();
    let mut seen = HashSet::new();
    let mut natives = Vec::new();
    for lib in &version.libraries {
        let plan = plan_library(lib, &env, dirs)?;
        if let Some((spec, path)) = plan.artifact {
            if seen.insert(path.clone()) {
                classpath.push(path);
            }
            specs.extend(spec);
        }
        if let Some((spec, exclude)) = plan.native {
            natives.push((spec.path.clone(), exclude));
            specs.push(spec);
        }
    }

    // У версий загрузчиков игра лежит под их id: Forge/NeoForge исключают из модулей
    // именно `${version_name}.jar` (так же делает официальный лаунчер).
    let client_jar = if version.id == mc {
        vanilla_jar
    } else {
        let own = dirs.client_jar(&version.id);
        let same =
            std::fs::metadata(&own).ok().map(|m| m.len()) == client.size && client.size.is_some();
        if !same {
            std::fs::create_dir_all(own.parent().unwrap())?;
            std::fs::copy(&vanilla_jar, &own).with_context(|| {
                format!("не удалось скопировать client.jar в {}", own.display())
            })?;
        }
        own
    };
    classpath.push(client_jar);

    if let Some(log) = version.logging.as_ref().and_then(|l| l.client.as_ref()) {
        specs.push(
            FileSpec::new(&log.file.url, dirs.log_config(&log.file.id))
                .sha1(Some(&log.file.sha1))
                .size(Some(log.file.size)),
        );
    }

    let index_ref = version
        .asset_index
        .as_ref()
        .context("в JSON версии нет assetIndex")?;
    let index: AssetIndex = net
        .cached_json(
            &FileSpec::new(&index_ref.url, dirs.asset_index(&index_ref.id))
                .sha1(Some(&index_ref.sha1))
                .size(Some(index_ref.size)),
        )
        .await
        .context("не удалось получить список ресурсов игры")?;
    let mut hashes = HashSet::new();
    for obj in index.objects.values() {
        if hashes.insert(obj.hash.clone()) && scam_core::paths::is_sha1(&obj.hash) {
            specs.push(
                FileSpec::new(
                    format!("{ASSETS_BASE}/{}/{}", &obj.hash[..2], obj.hash),
                    dirs.asset_object(&obj.hash),
                )
                .sha1(Some(&obj.hash))
                .size(Some(obj.size)),
            );
        }
    }

    progress(Progress::Stage("Загрузка файлов игры".into()));
    net.ensure_all(specs, opts.verify, progress).await?;

    let natives_dir = dirs.natives(&version.id, arch);
    let dir = natives_dir.clone();
    tokio::task::spawn_blocking(move || extract_natives(&dir, &natives)).await??;

    Ok(Prepared {
        version,
        env,
        java,
        classpath,
        natives_dir,
    })
}

async fn mojang_manifest(net: &Net, dirs: &GameDirs) -> Result<Manifest> {
    net.cached_json(&FileSpec::new(
        VERSION_MANIFEST,
        dirs.versions().join("version_manifest_v2.json"),
    ))
    .await
}

/// Загружает JSON версии и рекурсивно сливает с родителями (`inheritsFrom`).
fn load_version<'a>(
    net: &'a Net,
    dirs: &'a GameDirs,
    manifest: Option<&'a Manifest>,
    id: &'a str,
) -> Pin<Box<dyn Future<Output = Result<VersionJson>> + Send + 'a>> {
    Box::pin(async move {
        let path = dirs.version_json(id);
        let json: VersionJson = match manifest.and_then(|m| m.versions.iter().find(|v| v.id == id))
        {
            Some(v) => {
                net.cached_json(&FileSpec::new(&v.url, &path).sha1(Some(&v.sha1)))
                    .await?
            }
            None if path.is_file() => serde_json::from_slice(&std::fs::read(&path)?)
                .with_context(|| format!("повреждён {}", path.display()))?,
            None => bail!("версия {id} не найдена"),
        };
        match json.inherits_from.clone() {
            Some(parent) => {
                let parent = load_version(net, dirs, manifest, &parent).await?;
                Ok(json.merge_onto(parent))
            }
            None => Ok(json),
        }
    })
}

struct LibPlan {
    /// Файл для classpath (spec — если его нужно скачать).
    artifact: Option<(Option<FileSpec>, PathBuf)>,
    /// Старый формат натив: jar и что не распаковывать.
    native: Option<(FileSpec, Vec<String>)>,
}

/// Архитектура у нового формата натив задаётся суффиксом классификатора.
fn native_classifier_matches(classifier: &str, arch: &str) -> bool {
    if !classifier.starts_with("natives-") {
        return true;
    }
    if classifier.ends_with("-arm64") || classifier.ends_with("-aarch_64") {
        arch == "aarch64"
    } else if classifier.ends_with("-x86") {
        arch == "x86"
    } else if classifier.ends_with("-arm32") {
        false
    } else {
        arch == "x86_64"
    }
}

fn plan_library(lib: &Library, env: &Env, dirs: &GameDirs) -> Result<LibPlan> {
    let mut plan = LibPlan {
        artifact: None,
        native: None,
    };
    if !rules::allowed(&lib.rules, env) {
        return Ok(plan);
    }
    let maven = lib.maven();
    if let Some(c) = maven.as_ref().and_then(|m| m.classifier.as_deref())
        && !native_classifier_matches(c, env.arch)
    {
        return Ok(plan);
    }

    let artifact = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref());
    match artifact {
        Some(a) => {
            let rel = a
                .path
                .clone()
                .or_else(|| maven.as_ref().map(|m| m.path()))
                .with_context(|| format!("не понять путь библиотеки {}", lib.name))?;
            let path = safe_join(&dirs.libraries(), &rel)?;
            // Пустой url — файл создаётся установщиком (Forge), качать нечего.
            let spec = (!a.url.is_empty()).then(|| {
                FileSpec::new(&a.url, &path)
                    .sha1(a.sha1.as_deref())
                    .size(a.size)
            });
            plan.artifact = Some((spec, path));
        }
        None if lib.natives.is_none() => {
            let m = maven.with_context(|| format!("странное имя библиотеки {}", lib.name))?;
            let rel = m.path();
            let base = lib.url.as_deref().unwrap_or(LIBRARIES_BASE);
            let url = format!("{}/{rel}", base.trim_end_matches('/'));
            let path = safe_join(&dirs.libraries(), &rel)?;
            plan.artifact = Some((
                Some(
                    FileSpec::new(url, &path)
                        .sha1(lib.sha1.as_deref())
                        .size(lib.size),
                ),
                path,
            ));
        }
        None => {}
    }

    if let Some(natives) = &lib.natives
        && let Some(classifier) = natives.get(env.os)
    {
        let bits = if env.arch == "x86" { "32" } else { "64" };
        let classifier = classifier.replace("${arch}", bits);
        let a = lib
            .downloads
            .as_ref()
            .and_then(|d| d.classifiers.as_ref())
            .and_then(|c| c.get(&classifier))
            .with_context(|| format!("у {} нет натив {classifier}", lib.name))?;
        let rel = a
            .path
            .clone()
            .or_else(|| {
                lib.maven().map(|m| {
                    Maven {
                        classifier: Some(classifier.clone()),
                        ..m
                    }
                    .path()
                })
            })
            .with_context(|| format!("не понять путь натив {}", lib.name))?;
        let path = safe_join(&dirs.libraries(), &rel)?;
        plan.native = Some((
            FileSpec::new(&a.url, &path)
                .sha1(a.sha1.as_deref())
                .size(a.size),
            lib.extract
                .as_ref()
                .map(|e| e.exclude.clone())
                .unwrap_or_default(),
        ));
    }
    Ok(plan)
}

fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    if !scam_core::paths::is_safe_rel_path(rel) {
        bail!("небезопасный путь библиотеки: {rel}");
    }
    Ok(base.join(rel))
}

/// Распаковывает старые натив (`natives` в JSON) в свежую папку.
fn extract_natives(dir: &Path, jars: &[(PathBuf, Vec<String>)]) -> Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    for (jar, exclude) in jars {
        let file = std::fs::File::open(jar).with_context(|| format!("нет {}", jar.display()))?;
        let mut zip =
            zip::ZipArchive::new(file).with_context(|| format!("битый архив {}", jar.display()))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let name = entry.name().to_owned();
            if entry.is_dir()
                || exclude.iter().any(|e| name.starts_with(e.as_str()))
                || !scam_core::paths::is_safe_rel_path(&name)
            {
                continue;
            }
            let out = dir.join(&name);
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut f = std::fs::File::create(&out)?;
            std::io::copy(&mut entry, &mut f)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_arch_filter() {
        assert!(native_classifier_matches("natives-macos-arm64", "aarch64"));
        assert!(!native_classifier_matches("natives-macos", "aarch64"));
        assert!(native_classifier_matches("natives-macos", "x86_64"));
        assert!(native_classifier_matches("natives-windows", "x86_64"));
        assert!(!native_classifier_matches("natives-windows-x86", "x86_64"));
        assert!(native_classifier_matches("natives-windows-x86", "x86"));
        assert!(native_classifier_matches("sources", "x86_64"));
    }

    #[test]
    fn library_plans() {
        let dirs = GameDirs::new("/g");
        let env = Env {
            os: "osx",
            arch: "aarch64",
            os_version: String::new(),
            features: Default::default(),
        };
        let lib: Library = serde_json::from_str(
            r#"{"name":"org.lwjgl:lwjgl:3.3.1:natives-macos-arm64","rules":[{"action":"allow","os":{"name":"osx"}}],
               "downloads":{"artifact":{"path":"org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-macos-arm64.jar","url":"https://libraries.minecraft.net/x.jar","sha1":"a","size":1}}}"#,
        )
        .unwrap();
        assert!(plan_library(&lib, &env, &dirs).unwrap().artifact.is_some());

        let x64: Library = serde_json::from_str(
            r#"{"name":"org.lwjgl:lwjgl:3.3.1:natives-macos","downloads":{"artifact":{"url":"u","path":"p.jar"}}}"#,
        )
        .unwrap();
        assert!(plan_library(&x64, &env, &dirs).unwrap().artifact.is_none());

        let fabric: Library = serde_json::from_str(
            r#"{"name":"net.fabricmc:fabric-loader:0.16.14","url":"https://maven.fabricmc.net/"}"#,
        )
        .unwrap();
        let (spec, path) = plan_library(&fabric, &env, &dirs)
            .unwrap()
            .artifact
            .unwrap();
        assert_eq!(
            spec.unwrap().url,
            "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.14/fabric-loader-0.16.14.jar"
        );
        assert!(path.ends_with("net/fabricmc/fabric-loader/0.16.14/fabric-loader-0.16.14.jar"));

        let legacy: Library = serde_json::from_str(
            r#"{"name":"org.lwjgl:lwjgl:3.2.1","natives":{"osx":"natives-macos"},"extract":{"exclude":["META-INF/"]},
               "downloads":{"artifact":{"path":"a.jar","url":"u1"},"classifiers":{"natives-macos":{"path":"n.jar","url":"u2"}}}}"#,
        )
        .unwrap();
        let plan = plan_library(&legacy, &env, &dirs).unwrap();
        assert!(plan.artifact.is_some());
        let (spec, exclude) = plan.native.unwrap();
        assert_eq!(spec.url, "u2");
        assert_eq!(exclude, vec!["META-INF/"]);

        let evil: Library = serde_json::from_str(
            r#"{"name":"a:b:1","downloads":{"artifact":{"path":"../../etc/x","url":"u"}}}"#,
        )
        .unwrap();
        assert!(plan_library(&evil, &env, &dirs).is_err());
    }
}
