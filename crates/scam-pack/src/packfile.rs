//! `pack.toml` — описание сборки у автора.

use anyhow::{Context, Result, bail, ensure};
use scam_core::model::{GroupMode, LoaderKind};
use scam_core::paths::is_valid_id;
use scam_core::pattern::PatternSet;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "pack.toml";

/// Исключается всегда, даже если не указано в `exclude`.
const ALWAYS_EXCLUDE: &[&str] = &[
    "**/.DS_Store",
    "**/Thumbs.db",
    "**/desktop.ini",
    ".scam/**",
    "mods-clients/**",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    pack: PackMeta,
    #[serde(default, rename = "group")]
    groups: Vec<RawGroup>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackMeta {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub minecraft: String,
    pub loader: String,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub java: Option<u32>,
    #[serde(default)]
    pub memory_min: Option<u32>,
    #[serde(default)]
    pub memory_recommended: Option<u32>,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn default_source() -> String {
    ".".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGroup {
    id: String,
    #[serde(default)]
    name: Option<String>,
    mode: GroupMode,
    include: Vec<String>,
    #[serde(default)]
    revision: Option<u32>,
    #[serde(default)]
    roots: Vec<String>,
    #[serde(default)]
    prune: Vec<String>,
}

#[derive(Debug)]
pub struct GroupDef {
    pub id: String,
    pub name: Option<String>,
    pub mode: GroupMode,
    pub revision: u32,
    pub include: PatternSet,
    pub roots: Vec<String>,
    pub prune: Vec<String>,
}

#[derive(Debug)]
pub struct Pack {
    pub meta: PackMeta,
    pub loader: LoaderKind,
    /// Папка, из которой собирается сборка.
    pub source: PathBuf,
    pub icon: Option<PathBuf>,
    pub background: Option<PathBuf>,
    pub exclude: PatternSet,
    pub groups: Vec<GroupDef>,
}

fn expand_path(base: &Path, p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    base.join(p)
}

impl Pack {
    pub fn load(path: &Path) -> Result<Pack> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("не удалось прочитать {}", path.display()))?;
        let raw: Raw =
            toml::from_str(&text).with_context(|| format!("ошибка в {}", path.display()))?;
        let base = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        Self::from_raw(raw, base, path).with_context(|| format!("ошибка в {}", path.display()))
    }

    fn from_raw(raw: Raw, base: &Path, file: &Path) -> Result<Pack> {
        let meta = raw.pack;
        ensure!(
            is_valid_id(&meta.id),
            "pack.id «{}»: только латиница в нижнем регистре, цифры, «-» и «_»",
            meta.id
        );
        ensure!(!meta.name.trim().is_empty(), "pack.name пустое");
        ensure!(!meta.minecraft.trim().is_empty(), "pack.minecraft пустое");
        let loader: LoaderKind = meta.loader.parse().map_err(anyhow::Error::msg)?;
        if loader != LoaderKind::Vanilla && meta.loader_version.is_none() {
            bail!("для {} нужен pack.loader_version", loader.title());
        }
        if let (Some(min), Some(rec)) = (meta.memory_min, meta.memory_recommended) {
            ensure!(min <= rec, "memory_min больше memory_recommended");
        }

        let source = expand_path(base, &meta.source);
        ensure!(
            source.is_dir(),
            "папка сборки pack.source не найдена: {}",
            source.display()
        );
        let source = source.canonicalize()?;
        let icon = meta.icon.as_deref().map(|p| expand_path(base, p));
        let background = meta.background.as_deref().map(|p| expand_path(base, p));
        for p in icon.iter().chain(&background) {
            ensure!(p.is_file(), "файл не найден: {}", p.display());
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            ensure!(
                matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif"),
                "{}: картинка должна быть png, jpg, webp или gif",
                p.display()
            );
            ensure!(
                std::fs::metadata(p)?.len() <= 5 * 1024 * 1024,
                "{}: картинка больше 5 МБ",
                p.display()
            );
        }

        // pack.toml, иконка и фон внутри папки сборки в саму сборку не попадают.
        let mut exclude: Vec<String> = ALWAYS_EXCLUDE.iter().map(|s| s.to_string()).collect();
        for p in std::iter::once(file.to_path_buf())
            .chain(icon.clone())
            .chain(background.clone())
        {
            if let Ok(abs) = p.canonicalize()
                && let Ok(rel) = abs.strip_prefix(&source)
                && let Some(rel) = rel.to_str()
            {
                exclude.push(glob_escape(&rel.replace('\\', "/")));
            }
        }
        exclude.extend(meta.exclude.iter().cloned());
        let exclude = PatternSet::parse(&exclude)?;

        ensure!(!raw.groups.is_empty(), "нет ни одной группы [[group]]");
        let mut ids = HashSet::new();
        let mut groups = Vec::new();
        for g in raw.groups {
            ensure!(
                is_valid_id(&g.id),
                "group.id «{}»: только латиница, цифры, «-», «_»",
                g.id
            );
            ensure!(ids.insert(g.id.clone()), "группа «{}» указана дважды", g.id);
            ensure!(!g.include.is_empty(), "у группы «{}» пустой include", g.id);
            match g.mode {
                GroupMode::Sync => {
                    ensure!(
                        g.roots.is_empty(),
                        "группа «{}»: roots бывает только у mode = \"once\"",
                        g.id
                    );
                    ensure!(
                        g.revision.is_none(),
                        "группа «{}»: revision бывает только у mode = \"once\"",
                        g.id
                    );
                }
                GroupMode::Once => {
                    ensure!(
                        g.prune.is_empty(),
                        "группа «{}»: prune бывает только у mode = \"sync\"",
                        g.id
                    );
                }
            }
            let revision = g.revision.unwrap_or(1);
            ensure!(revision >= 1, "группа «{}»: revision должна быть ≥ 1", g.id);
            for r in &g.roots {
                ensure!(
                    scam_core::paths::is_safe_rel_path(r),
                    "группа «{}»: неверный корень «{r}»",
                    g.id
                );
            }
            let prune =
                PatternSet::parse(&g.prune).with_context(|| format!("группа «{}», prune", g.id))?;
            for p in &prune.0 {
                // Лаунчер обходит только папку из начала маски — без неё prune не сработает.
                let Some(root) = p.static_root() else {
                    bail!(
                        "группа «{}»: prune «{}» должен начинаться с папки, например \"mods/*.jar\"",
                        g.id,
                        p.as_str()
                    );
                };
                let top = root.split('/').next().unwrap_or("");
                ensure!(
                    top != ".scam" && top != "mods-clients",
                    "группа «{}»: prune не может касаться {top}/",
                    g.id
                );
            }
            groups.push(GroupDef {
                include: PatternSet::parse(&g.include)
                    .with_context(|| format!("группа «{}», include", g.id))?,
                id: g.id,
                name: g.name,
                mode: g.mode,
                revision,
                roots: g.roots,
                prune: g.prune,
            });
        }

        Ok(Pack {
            meta,
            loader,
            source,
            icon,
            background,
            exclude,
            groups,
        })
    }
}

fn glob_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub struct InitOptions {
    pub id: String,
    pub name: String,
    pub minecraft: String,
    pub loader: LoaderKind,
    pub loader_version: Option<String>,
    pub source: String,
}

pub fn template(o: &InitOptions) -> String {
    let loader_version = match &o.loader_version {
        Some(v) => format!("loader_version = \"{v}\""),
        None if o.loader == LoaderKind::Vanilla => "# loader_version не нужен для vanilla".into(),
        None => {
            "loader_version = \"УКАЖИ_ВЕРСИЮ\"   # например 0.16.14 для Fabric, 47.4.0 для Forge"
                .into()
        }
    };
    format!(
        r#"# Описание сборки для scam-pack. Подробности — docs/PUBLISHING.md.

[pack]
id = "{id}"                      # латиница, цифры, «-», «_»; менять нельзя
name = "{name}"
# description = "Короткое описание для лаунчера"
minecraft = "{mc}"
loader = "{loader}"                   # vanilla | fabric | quilt | forge | neoforge
{loader_version}
memory_recommended = 4096        # МБ, подсказка для ползунка памяти
# memory_min = 3072
# java = 21                      # обычно не нужно: берётся из данных Mojang
source = "{source}"                      # папка с файлами сборки (как .minecraft)
# icon = "icon.png"
# background = "background.jpg"

# Что не попадает в сборку. glob: «*» — внутри папки, «**» — на любую глубину.
# Регулярное выражение — с префиксом «re:», проверяется по пути вида «config/x.toml».
exclude = [
  "saves/**", "logs/**", "crash-reports/**", "screenshots/**", "downloads/**",
  ".fabric/**", ".mixin.out/**", "local/**", "journeymap/data/**", "xaero/**",
  "*.log", "usercache.json", "usernamecache.json", "realms_persistence.json",
  "command_history.txt", "patchouli_data.json",
]

# Группы проверяются по порядку, файл попадает в первую подходящую.
# Файл, не попавший ни в одну группу, — ошибка публикации (чтобы ничего не утекло случайно).
#
# mode = "sync"  — всегда как в сборке: изменённые и удалённые файлы восстанавливаются.
#   prune        — маски, в пределах которых лишние файлы у игрока удаляются.
# mode = "once"  — ставится один раз, дальше это файлы игрока. Ставится заново, если:
#   · игрок удалил корень группы целиком (папку shaderpacks/, файл options.txt, …);
#   · ты увеличил revision — тогда у всех перезапишется один раз, со старого сделается бэкап;
#   · игрок сам нажал «Восстановить файлы сборки».
#   roots        — корни вручную (по умолчанию берутся из include).

[[group]]
id = "mods"
name = "Моды"
mode = "sync"
include = ["mods/*.jar"]
prune = ["mods/*.jar"]

[[group]]
id = "controls"
name = "Настройки и управление"
mode = "once"
revision = 1
include = ['re:^options(of|shaders)?\.txt$']

[[group]]
id = "configs"
name = "Конфиги модов"
mode = "once"
revision = 1
include = ["config/**", "defaultconfigs/**", "kubejs/**"]

[[group]]
id = "shaders"
name = "Шейдеры"
mode = "once"
revision = 1
include = ["shaderpacks/**"]

[[group]]
id = "resourcepacks"
name = "Текстуры"
mode = "once"
revision = 1
include = ["resourcepacks/**"]

[[group]]
id = "emotes"
name = "Эмоции"
mode = "once"
revision = 1
include = ["emotes/**"]

# Список серверов в игре:
# [[group]]
# id = "servers"
# name = "Список серверов"
# mode = "once"
# include = ["servers.dat"]

# Конфиги, которые обязаны совпадать с сервером:
# [[group]]
# id = "server-configs"
# mode = "sync"
# include = ['re:^config/(create|ftbchunks)-common\.toml$']
"#,
        id = o.id,
        name = o.name,
        mc = o.minecraft,
        loader = format!("{:?}", o.loader).to_lowercase(),
        loader_version = loader_version,
        source = o.source,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_str(toml_text: &str) -> Result<Pack> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, toml_text)?;
        let res = Pack::load(&path);
        drop(dir);
        res
    }

    #[test]
    fn template_is_valid() {
        let text = template(&InitOptions {
            id: "techno".into(),
            name: "Техно".into(),
            minecraft: "1.20.1".into(),
            loader: LoaderKind::NeoForge,
            loader_version: Some("47.1.106".into()),
            source: ".".into(),
        });
        let pack = load_str(&text).unwrap();
        assert_eq!(pack.loader, LoaderKind::NeoForge);
        assert_eq!(pack.groups.len(), 6);
        assert!(pack.exclude.is_match("pack.toml"));
        assert!(pack.exclude.is_match("saves/world/level.dat"));
        assert!(pack.exclude.is_match("mods-clients/x.jar"));
    }

    #[test]
    fn validation_errors() {
        let base = |extra: &str| {
            format!(
                "[pack]\nid=\"p\"\nname=\"P\"\nminecraft=\"1.20.1\"\nloader=\"fabric\"\nloader_version=\"0.16\"\n{extra}"
            )
        };
        let err = |t: String| format!("{:#}", load_str(&t).unwrap_err());

        assert!(err(base("")).contains("нет ни одной группы"));
        assert!(
            err(base(
                "[[group]]\nid=\"a\"\nmode=\"sync\"\ninclude=[\"x\"]\nroots=[\"x\"]"
            ))
            .contains("roots")
        );
        assert!(
            err(base(
                "[[group]]\nid=\"a\"\nmode=\"once\"\ninclude=[\"x\"]\nprune=[\"x\"]"
            ))
            .contains("prune")
        );
        assert!(
            err(base(
                "[[group]]\nid=\"a\"\nmode=\"sync\"\ninclude=[\"x\"]\nprune=[\"*.jar\"]"
            ))
            .contains("начинаться с папки")
        );
        assert!(
            err(base(
                "[[group]]\nid=\"a\"\nmode=\"sync\"\ninclude=[\"x\"]\nprune=[\"mods-clients/*.jar\"]"
            ))
            .contains("mods-clients")
        );
        assert!(
            err(base(
                "[[group]]\nid=\"a\"\nmode=\"once\"\ninclude=[\"re:(\"]"
            ))
            .contains("регулярное")
        );
        assert!(err(base("[[group]]\nid=\"a\"\nmode=\"once\"\ninclude=[\"x\"]\n[[group]]\nid=\"a\"\nmode=\"once\"\ninclude=[\"y\"]")).contains("дважды"));
        assert!(
            err(base(
                "[[group]]\nid=\"a\"\nmode=\"once\"\ninclude=[\"x\"]\ntypo=1"
            ))
            .contains("typo")
        );
        assert!(err("[pack]\nid=\"p\"\nname=\"P\"\nminecraft=\"1\"\nloader=\"forge\"\n[[group]]\nid=\"a\"\nmode=\"sync\"\ninclude=[\"x\"]".into()).contains("loader_version"));
        assert!(err("[pack]\nid=\"Bad\"\nname=\"P\"\nminecraft=\"1\"\nloader=\"vanilla\"\n[[group]]\nid=\"a\"\nmode=\"sync\"\ninclude=[\"x\"]".into()).contains("pack.id"));
    }
}
