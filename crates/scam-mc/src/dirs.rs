use std::path::PathBuf;

/// Раскладка общей папки лаунчера: версии, библиотеки, ассеты и Java общие для всех сборок.
#[derive(Debug, Clone)]
pub struct GameDirs {
    pub root: PathBuf,
}

impl GameDirs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `%APPDATA%\ScamLauncher`, `~/Library/Application Support/ScamLauncher`, `~/.local/share/ScamLauncher`.
    pub fn default_root() -> PathBuf {
        ::dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ScamLauncher")
    }

    pub fn versions(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn version_json(&self, id: &str) -> PathBuf {
        self.versions().join(id).join(format!("{id}.json"))
    }

    pub fn client_jar(&self, id: &str) -> PathBuf {
        self.versions().join(id).join(format!("{id}.jar"))
    }

    pub fn libraries(&self) -> PathBuf {
        self.root.join("libraries")
    }

    pub fn assets(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn asset_index(&self, id: &str) -> PathBuf {
        self.assets().join("indexes").join(format!("{id}.json"))
    }

    pub fn asset_object(&self, hash: &str) -> PathBuf {
        self.assets().join("objects").join(&hash[..2]).join(hash)
    }

    pub fn log_config(&self, id: &str) -> PathBuf {
        self.assets().join("log_configs").join(id)
    }

    pub fn runtime(&self) -> PathBuf {
        self.root.join("runtime")
    }

    pub fn natives(&self, version: &str, arch: &str) -> PathBuf {
        self.root.join("natives").join(format!("{version}-{arch}"))
    }

    pub fn instance(&self, pack: &str) -> PathBuf {
        self.root.join("instances").join(pack)
    }
}
