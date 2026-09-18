//! Маски путей для групп: glob (`mods/*.jar`, `config/**`) или регулярка с префиксом `re:`.
//!
//! Пути всегда относительные, с `/`. В glob `*` не переходит через `/`, `**` — переходит.
//! Регистр в glob не важен (`mods/xaero*.jar` найдёт и `Xaeros_Minimap.jar`); в регулярках — как написано.

use globset::{GlobBuilder, GlobMatcher};
use regex::Regex;

#[derive(Debug, thiserror::Error)]
pub enum PatternError {
    #[error("неверная маска «{0}»: {1}")]
    Glob(String, globset::Error),
    #[error("неверное регулярное выражение «{0}»: {1}")]
    Regex(String, regex::Error),
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Glob { src: String, matcher: GlobMatcher },
    Regex { src: String, re: Regex },
}

const GLOB_META: &[char] = &['*', '?', '[', ']', '{', '}', '\\'];

impl Pattern {
    pub fn parse(src: &str) -> Result<Self, PatternError> {
        if let Some(re) = src.strip_prefix("re:") {
            let re = Regex::new(re).map_err(|e| PatternError::Regex(src.into(), e))?;
            return Ok(Pattern::Regex {
                src: src.into(),
                re,
            });
        }
        let glob = GlobBuilder::new(src)
            .literal_separator(true)
            .case_insensitive(true)
            .backslash_escape(true)
            .build()
            .map_err(|e| PatternError::Glob(src.into(), e))?;
        Ok(Pattern::Glob {
            src: src.into(),
            matcher: glob.compile_matcher(),
        })
    }

    pub fn as_str(&self) -> &str {
        match self {
            Pattern::Glob { src, .. } | Pattern::Regex { src, .. } => src,
        }
    }

    pub fn is_match(&self, path: &str) -> bool {
        match self {
            Pattern::Glob { matcher, .. } => matcher.is_match(path),
            Pattern::Regex { re, .. } => re.is_match(path),
        }
    }

    /// Неизменяемая часть glob до первого спецсимвола: `shaderpacks/**` → `shaderpacks`,
    /// `options.txt` → `options.txt`. Для регулярок и масок вида `*.txt` — `None`.
    pub fn static_root(&self) -> Option<String> {
        let Pattern::Glob { src, .. } = self else {
            return None;
        };
        let parts: Vec<&str> = src
            .split('/')
            .take_while(|part| !part.contains(GLOB_META))
            .collect();
        if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
            None
        } else {
            Some(parts.join("/"))
        }
    }

    /// Совпадает ли маска вида `dir/**` с целой папкой — чтобы не обходить исключённые папки.
    pub fn covers_dir(&self, dir: &str) -> bool {
        match self {
            Pattern::Glob { src, .. } => src
                .strip_suffix("/**")
                .and_then(|prefix| Pattern::parse(prefix).ok())
                .is_some_and(|p| p.is_match(dir)),
            Pattern::Regex { .. } => false,
        }
    }
}

/// Набор масок: совпадение с любой.
#[derive(Debug, Clone, Default)]
pub struct PatternSet(pub Vec<Pattern>);

impl PatternSet {
    pub fn parse<S: AsRef<str>>(items: &[S]) -> Result<Self, PatternError> {
        items
            .iter()
            .map(|s| Pattern::parse(s.as_ref()))
            .collect::<Result<_, _>>()
            .map(PatternSet)
    }

    pub fn is_match(&self, path: &str) -> bool {
        self.0.iter().any(|p| p.is_match(path))
    }

    pub fn first_match(&self, path: &str) -> Option<&Pattern> {
        self.0.iter().find(|p| p.is_match(path))
    }

    pub fn covers_dir(&self, dir: &str) -> bool {
        self.0.iter().any(|p| p.covers_dir(dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, path: &str) -> bool {
        Pattern::parse(p).unwrap().is_match(path)
    }

    #[test]
    fn glob_semantics() {
        assert!(m("mods/*.jar", "mods/create.jar"));
        assert!(!m("mods/*.jar", "mods/sub/create.jar"));
        assert!(!m("mods/*.jar", "mods/create.jar.disabled"));
        assert!(m("config/**", "config/a/b/c.toml"));
        assert!(m("shaderpacks/**", "shaderpacks/BSL.zip"));
        assert!(m("options.txt", "options.txt"));
        assert!(!m("options.txt", "config/options.txt"));
        assert!(m("**/.DS_Store", ".DS_Store"));
        assert!(m("**/.DS_Store", "a/b/.DS_Store"));
        assert!(m(
            "mods/xaero*minimap*.jar",
            "mods/Xaeros_Minimap_25.2_Fabric.jar"
        ));
        assert!(m("mods/*.jar", "mods/UPPER.JAR"));
    }

    #[test]
    fn regex_semantics() {
        let p = r"re:^options(of|shaders)?\.txt$";
        assert!(m(p, "options.txt"));
        assert!(m(p, "optionsof.txt"));
        assert!(m(p, "optionsshaders.txt"));
        assert!(!m(p, "options.txt.bak"));
        assert!(Pattern::parse("re:(").is_err());
    }

    #[test]
    fn static_roots() {
        let r = |p: &str| Pattern::parse(p).unwrap().static_root();
        assert_eq!(r("shaderpacks/**").as_deref(), Some("shaderpacks"));
        assert_eq!(r("config/create/*.toml").as_deref(), Some("config/create"));
        assert_eq!(r("options.txt").as_deref(), Some("options.txt"));
        assert_eq!(r("*.txt"), None);
        assert_eq!(r("**/x"), None);
        assert_eq!(r("re:^mods/"), None);
    }

    #[test]
    fn dir_cover() {
        let p = Pattern::parse("saves/**").unwrap();
        assert!(p.covers_dir("saves"));
        assert!(!p.covers_dir("saves2"));
        assert!(!Pattern::parse("saves/*.dat").unwrap().covers_dir("saves"));
    }
}
