//! Что лаунчер помнит о скачанной сборке — чтобы показывать и запускать её,
//! даже если автор удалил её с сервера.

use scam_core::model::{BuildManifest, IndexPack};
use scam_core::sync::STATE_DIR;
use std::path::Path;

const PACK_FILE: &str = "pack.json";
const MANIFEST_FILE: &str = "manifest.json";

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(value)?)?;
    std::fs::rename(&tmp, path)
}

/// Сохраняет описание сборки и манифест установленного билда.
pub fn save(instance: &Path, pack: &IndexPack, manifest: &BuildManifest) {
    let dir = instance.join(STATE_DIR);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = write_json(&dir.join(PACK_FILE), pack);
    let _ = write_json(&dir.join(MANIFEST_FILE), manifest);
}

pub fn load_pack(instance: &Path) -> Option<IndexPack> {
    serde_json::from_slice(&std::fs::read(instance.join(STATE_DIR).join(PACK_FILE)).ok()?).ok()
}

pub fn load_manifest(instance: &Path) -> Option<BuildManifest> {
    let m: BuildManifest =
        serde_json::from_slice(&std::fs::read(instance.join(STATE_DIR).join(MANIFEST_FILE)).ok()?)
            .ok()?;
    scam_core::remote::validate_manifest(&m).ok()?;
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scam_core::model::{Channels, Loader, LoaderKind, Memory, SCHEMA};

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_pack(dir.path()).is_none());
        let pack = IndexPack {
            id: "p".into(),
            name: "P".into(),
            description: None,
            minecraft: "1.20.1".into(),
            loader: LoaderKind::Fabric,
            icon: None,
            background: None,
            server: None,
            channels: Channels::default(),
            updated: chrono::Utc::now(),
        };
        let manifest = BuildManifest {
            schema: SCHEMA,
            pack: "p".into(),
            name: "P".into(),
            build: 3,
            version: "1.0".into(),
            created: chrono::Utc::now(),
            minecraft: "1.20.1".into(),
            loader: Loader {
                kind: LoaderKind::Fabric,
                version: Some("0.16".into()),
            },
            java: None,
            memory: Memory::default(),
            changelog: None,
            groups: vec![],
            files: vec![],
        };
        save(dir.path(), &pack, &manifest);
        assert_eq!(load_pack(dir.path()).unwrap().id, "p");
        assert_eq!(load_manifest(dir.path()).unwrap().build, 3);
    }
}
