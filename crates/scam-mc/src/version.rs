//! JSON версии Minecraft (и профилей загрузчиков, которые наследуются от неё).

use crate::rules::Rule;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionJson {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Arguments>,
    /// Старый формат аргументов (одной строкой).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_arguments: Option<String>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<Downloads>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_version: Option<JavaVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<Logging>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Какой client.jar использовать (у загрузчиков — от родительской версии).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jar: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Arg>,
    #[serde(default)]
    pub jvm: Vec<Arg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Arg {
    Plain(String),
    Ruled { rules: Vec<Rule>, value: ArgValue },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArgValue {
    One(String),
    Many(Vec<String>),
}

impl ArgValue {
    pub fn values(&self) -> Vec<&str> {
        match self {
            ArgValue::One(s) => vec![s],
            ArgValue::Many(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<LibDownloads>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Старый формат натив: ОС → классификатор (`natives-windows-${arch}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natives: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<Extract>,
    /// Базовый адрес maven-репозитория (так пишут Fabric/Quilt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexRef {
    pub id: String,
    pub url: String,
    pub sha1: String,
    pub size: u64,
    #[serde(default)]
    pub total_size: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Downloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<Artifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    #[serde(default)]
    pub component: Option<String>,
    pub major_version: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Logging {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<LoggingClient>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingClient {
    pub argument: String,
    pub file: LogFile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogFile {
    pub id: String,
    pub url: String,
    pub sha1: String,
    pub size: u64,
}

/// Maven-координата `group:artifact:version[:classifier][@ext]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Maven {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: Option<String>,
    pub ext: String,
}

impl Maven {
    pub fn parse(name: &str) -> Option<Maven> {
        let (coords, ext) = match name.split_once('@') {
            Some((c, e)) => (c, e.to_owned()),
            None => (name, "jar".to_owned()),
        };
        let mut parts = coords.split(':');
        let group = parts.next()?.to_owned();
        let artifact = parts.next()?.to_owned();
        let version = parts.next()?.to_owned();
        let classifier = parts.next().map(str::to_owned);
        if parts.next().is_some() || group.is_empty() || artifact.is_empty() || version.is_empty() {
            return None;
        }
        Some(Maven {
            group,
            artifact,
            version,
            classifier,
            ext,
        })
    }

    /// `org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-macos.jar`
    pub fn path(&self) -> String {
        let classifier = self
            .classifier
            .as_deref()
            .map(|c| format!("-{c}"))
            .unwrap_or_default();
        format!(
            "{}/{}/{}/{}-{}{}.{}",
            self.group.replace('.', "/"),
            self.artifact,
            self.version,
            self.artifact,
            self.version,
            classifier,
            self.ext
        )
    }

    /// Ключ для дедупликации: одна и та же библиотека разных версий — один ключ.
    pub fn key(&self) -> String {
        match &self.classifier {
            Some(c) => format!("{}:{}:{c}", self.group, self.artifact),
            None => format!("{}:{}", self.group, self.artifact),
        }
    }
}

impl Library {
    pub fn maven(&self) -> Option<Maven> {
        Maven::parse(&self.name)
    }
}

impl VersionJson {
    /// Сливает профиль загрузчика (`self`) с родительской версией.
    /// Библиотеки ребёнка важнее: одинаковая библиотека родителя (другой версии) выбрасывается.
    pub fn merge_onto(self, parent: VersionJson) -> VersionJson {
        let child_keys: HashSet<String> = self
            .libraries
            .iter()
            .filter_map(|l| l.maven().map(|m| m.key()))
            .collect();
        let mut libraries = self.libraries;
        libraries.extend(
            parent
                .libraries
                .into_iter()
                .filter(|l| l.maven().is_none_or(|m| !child_keys.contains(&m.key()))),
        );

        let arguments = match (parent.arguments, self.arguments) {
            (Some(mut p), Some(c)) => {
                p.game.extend(c.game);
                p.jvm.extend(c.jvm);
                Some(p)
            }
            (p, c) => c.or(p),
        };

        VersionJson {
            jar: self.jar.or(parent.jar).or(Some(parent.id)),
            id: self.id,
            inherits_from: None,
            main_class: self.main_class.or(parent.main_class),
            arguments,
            minecraft_arguments: self.minecraft_arguments.or(parent.minecraft_arguments),
            libraries,
            asset_index: self.asset_index.or(parent.asset_index),
            assets: self.assets.or(parent.assets),
            downloads: self.downloads.or(parent.downloads),
            java_version: self.java_version.or(parent.java_version),
            logging: self.logging.or(parent.logging),
            kind: self.kind.or(parent.kind),
        }
    }

    /// Есть ли у версии нативные библиотеки под arm64 для macOS.
    pub fn has_macos_arm64_natives(&self) -> bool {
        self.libraries
            .iter()
            .any(|l| l.name.contains("natives-macos-arm64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_paths() {
        let m = Maven::parse("org.lwjgl:lwjgl:3.3.1:natives-macos-arm64").unwrap();
        assert_eq!(
            m.path(),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-macos-arm64.jar"
        );
        assert_eq!(m.key(), "org.lwjgl:lwjgl:natives-macos-arm64");
        let m = Maven::parse("net.fabricmc:fabric-loader:0.16.14").unwrap();
        assert_eq!(
            m.path(),
            "net/fabricmc/fabric-loader/0.16.14/fabric-loader-0.16.14.jar"
        );
        let m = Maven::parse("de.oceanlabs.mcp:mcp_config:1.20.1-20230612.114412@zip").unwrap();
        assert_eq!(
            m.path(),
            "de/oceanlabs/mcp/mcp_config/1.20.1-20230612.114412/mcp_config-1.20.1-20230612.114412.zip"
        );
        assert!(Maven::parse("broken").is_none());
    }

    #[test]
    fn parses_args_and_merges() {
        let parent: VersionJson = serde_json::from_str(
            r#"{"id":"1.20.1","mainClass":"net.minecraft.client.main.Main",
                "arguments":{"game":["--username","${auth_player_name}",
                    {"rules":[{"action":"allow","features":{"is_demo_user":true}}],"value":"--demo"}],
                  "jvm":[{"rules":[{"action":"allow","os":{"name":"osx"}}],"value":["-XstartOnFirstThread"]},"-cp","${classpath}"]},
                "libraries":[{"name":"org.ow2.asm:asm:9.3"},{"name":"com.mojang:brigadier:1.1.8"}],
                "assetIndex":{"id":"5","url":"u","sha1":"s","size":1},
                "javaVersion":{"component":"java-runtime-gamma","majorVersion":17}}"#,
        )
        .unwrap();
        let child: VersionJson = serde_json::from_str(
            r#"{"id":"fabric-loader-0.16-1.20.1","inheritsFrom":"1.20.1",
                "mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient",
                "arguments":{"game":[],"jvm":["-DFabricMcEmu= net.minecraft.client.main.Main "]},
                "libraries":[{"name":"org.ow2.asm:asm:9.6","url":"https://maven.fabricmc.net/"}]}"#,
        )
        .unwrap();
        let m = child.merge_onto(parent);
        assert_eq!(m.id, "fabric-loader-0.16-1.20.1");
        assert_eq!(m.jar.as_deref(), Some("1.20.1"));
        assert_eq!(
            m.main_class.as_deref(),
            Some("net.fabricmc.loader.impl.launch.knot.KnotClient")
        );
        let libs: Vec<&str> = m.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            libs,
            vec!["org.ow2.asm:asm:9.6", "com.mojang:brigadier:1.1.8"]
        );
        let args = m.arguments.unwrap();
        assert_eq!(args.jvm.len(), 4);
        assert_eq!(args.game.len(), 3);
        assert_eq!(m.java_version.unwrap().major_version, 17);
    }
}
