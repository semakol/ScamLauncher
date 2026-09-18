//! Достаёт id модов из jar: Fabric, Quilt, Forge и NeoForge.

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// id модов в jar. Для не-модов и битых архивов — пустой список.
pub fn mod_ids_from_jar(path: &Path) -> Vec<String> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(mut zip) = zip::ZipArchive::new(file) else {
        return Vec::new();
    };

    let mut ids = Vec::new();
    if let Some(text) = read_entry(&mut zip, "fabric.mod.json")
        && let Some(id) = json_str(&text, &["id"])
    {
        ids.push(id);
    }
    if let Some(text) = read_entry(&mut zip, "quilt.mod.json")
        && let Some(id) = json_str(&text, &["quilt_loader", "id"])
    {
        ids.push(id);
    }
    for name in ["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
        if let Some(text) = read_entry(&mut zip, name) {
            ids.extend(toml_mod_ids(&text));
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn read_entry(zip: &mut zip::ZipArchive<File>, name: &str) -> Option<String> {
    let mut entry = zip.by_name(name).ok()?;
    if entry.size() > 1024 * 1024 {
        return None;
    }
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    Some(text.trim_start_matches('\u{feff}').to_owned())
}

fn json_str(text: &str, path: &[&str]) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(text).ok()?;
    for key in path {
        value = value.get(key)?.clone();
    }
    value.as_str().map(str::to_owned).filter(|s| !s.is_empty())
}

fn toml_mod_ids(text: &str) -> Vec<String> {
    let Ok(table) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    table
        .get("mods")
        .and_then(|m| m.as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("modId")?.as_str())
        .filter(|id| !id.is_empty() && !id.contains("${"))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn jar(entries: &[(&str, &str)]) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut zip = zip::ZipWriter::new(file.reopen().unwrap());
        for (name, body) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        file
    }

    #[test]
    fn fabric() {
        let f = jar(&[("fabric.mod.json", r#"{"schemaVersion":1,"id":"sodium"}"#)]);
        assert_eq!(mod_ids_from_jar(f.path()), vec!["sodium"]);
    }

    #[test]
    fn quilt() {
        let f = jar(&[("quilt.mod.json", r#"{"quilt_loader":{"id":"qsl"}}"#)]);
        assert_eq!(mod_ids_from_jar(f.path()), vec!["qsl"]);
    }

    #[test]
    fn forge_multiple_mods() {
        let toml = r#"
modLoader = "javafml"
loaderVersion = "[47,)"
[[mods]]
modId = "create"
version = "${file.jarVersion}"
[[mods]]
modId = "flywheel"
"#;
        let f = jar(&[("META-INF/mods.toml", toml)]);
        assert_eq!(mod_ids_from_jar(f.path()), vec!["create", "flywheel"]);
    }

    #[test]
    fn neoforge_and_garbage() {
        let f = jar(&[("META-INF/neoforge.mods.toml", "[[mods]]\nmodId=\"jei\"")]);
        assert_eq!(mod_ids_from_jar(f.path()), vec!["jei"]);

        let f = jar(&[("fabric.mod.json", "not json")]);
        assert!(mod_ids_from_jar(f.path()).is_empty());

        let not_zip = tempfile::NamedTempFile::new().unwrap();
        assert!(mod_ids_from_jar(not_zip.path()).is_empty());
    }
}
