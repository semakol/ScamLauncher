//! Правила Mojang (`rules`) у библиотек и аргументов.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Где и как запускается игра. `arch` — архитектура выбранной Java (на Mac с M-чипом
/// старые версии идут через Rosetta, тогда здесь `x86_64`).
#[derive(Debug, Clone)]
pub struct Env {
    /// `osx` | `windows` | `linux` — как в JSON Mojang.
    pub os: &'static str,
    /// `x86_64` | `aarch64` | `x86`.
    pub arch: &'static str,
    pub os_version: String,
    pub features: BTreeMap<String, bool>,
}

impl Env {
    pub fn host() -> Self {
        Self::with_arch(host_arch())
    }

    pub fn with_arch(arch: &'static str) -> Self {
        Self {
            os: host_os(),
            arch,
            os_version: os_version(),
            features: BTreeMap::new(),
        }
    }
}

pub fn host_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "osx",
        "windows" => "windows",
        _ => "linux",
    }
}

pub fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86" => "x86",
        _ => "x86_64",
    }
}

fn os_version() -> String {
    // Нужна только для правила «Windows 10» у старых версий.
    #[cfg(windows)]
    {
        if let Ok(out) = std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(v) = text.split("Version ").nth(1) {
                return v.trim().trim_end_matches(']').to_owned();
            }
        }
    }
    String::new()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<BTreeMap<String, bool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
}

impl Rule {
    fn matches(&self, env: &Env) -> bool {
        if let Some(os) = &self.os {
            if let Some(name) = &os.name
                && name != env.os
            {
                return false;
            }
            if let Some(arch) = &os.arch
                && !arch_matches(arch, env.arch)
            {
                return false;
            }
            if let Some(re) = &os.version {
                match regex::Regex::new(re) {
                    Ok(re) if re.is_match(&env.os_version) => {}
                    _ => return false,
                }
            }
        }
        if let Some(features) = &self.features {
            for (k, v) in features {
                if env.features.get(k).copied().unwrap_or(false) != *v {
                    return false;
                }
            }
        }
        true
    }
}

fn arch_matches(rule: &str, arch: &str) -> bool {
    match rule {
        "x86" => arch == "x86",
        "x86_64" | "amd64" => arch == "x86_64",
        "arm64" | "aarch64" => arch == "aarch64",
        other => other == arch,
    }
}

/// Пустые правила — разрешено; иначе побеждает последнее подходящее правило.
pub fn allowed(rules: &[Rule], env: &Env) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allow = false;
    for r in rules {
        if r.matches(env) {
            allow = r.action == Action::Allow;
        }
    }
    allow
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(os: &'static str, arch: &'static str) -> Env {
        Env {
            os,
            arch,
            os_version: "10.0.19045".into(),
            features: BTreeMap::new(),
        }
    }

    fn rules(json: &str) -> Vec<Rule> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn os_rules() {
        let only_mac = rules(r#"[{"action":"allow","os":{"name":"osx"}}]"#);
        assert!(allowed(&only_mac, &env("osx", "aarch64")));
        assert!(!allowed(&only_mac, &env("windows", "x86_64")));

        let not_mac = rules(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#);
        assert!(!allowed(&not_mac, &env("osx", "x86_64")));
        assert!(allowed(&not_mac, &env("linux", "x86_64")));

        assert!(allowed(&[], &env("linux", "x86_64")));
    }

    #[test]
    fn arch_and_version_rules() {
        let x86 = rules(r#"[{"action":"allow","os":{"arch":"x86"}}]"#);
        assert!(allowed(&x86, &env("windows", "x86")));
        assert!(!allowed(&x86, &env("windows", "x86_64")));

        let win10 = rules(r#"[{"action":"allow","os":{"name":"windows","version":"^10\\."}}]"#);
        assert!(allowed(&win10, &env("windows", "x86_64")));
        let mut e = env("windows", "x86_64");
        e.os_version = "6.1".into();
        assert!(!allowed(&win10, &e));
    }

    #[test]
    fn feature_rules() {
        let demo = rules(r#"[{"action":"allow","features":{"is_demo_user":true}}]"#);
        let mut e = env("linux", "x86_64");
        assert!(!allowed(&demo, &e));
        e.features.insert("is_demo_user".into(), true);
        assert!(allowed(&demo, &e));
    }
}
