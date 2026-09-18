//! Раскладка файлов в публичной папке и проверка путей.

pub const INDEX_FILE: &str = "index.json";
pub const NEWS_FILE: &str = "news.json";
pub const OBJECTS_DIR: &str = "objects";
pub const PACKS_DIR: &str = "packs";

pub fn pack_dir(pack: &str) -> String {
    format!("{PACKS_DIR}/{pack}")
}

pub fn builds_dir(pack: &str) -> String {
    format!("{PACKS_DIR}/{pack}/builds")
}

pub fn build_file(pack: &str, build: u64) -> String {
    format!("{PACKS_DIR}/{pack}/builds/{build}.json")
}

pub fn object_dir(sha1: &str) -> String {
    format!("{OBJECTS_DIR}/{}", &sha1[..2])
}

pub fn object_file(sha1: &str) -> String {
    format!("{OBJECTS_DIR}/{}/{sha1}", &sha1[..2])
}

/// id сборки или группы: латиница в нижнем регистре, цифры, `-`, `_`.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && id.as_bytes()[0].is_ascii_alphanumeric()
}

pub fn is_sha1(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Безопасный относительный путь: `/`-разделители, без `..`, `.`, пустых частей,
/// абсолютных путей и букв дисков. Всё, что лаунчер пишет на диск игрока, проходит эту проверку.
pub fn is_safe_rel_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.contains(':')
        && !p.contains('\0')
        && p.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(is_valid_id("techno-2"));
        assert!(is_valid_id("a_b"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("-x"));
        assert!(!is_valid_id("Техно"));
        assert!(!is_valid_id("A"));
    }

    #[test]
    fn safe_paths() {
        assert!(is_safe_rel_path("mods/create.jar"));
        assert!(is_safe_rel_path("options.txt"));
        assert!(!is_safe_rel_path("../evil"));
        assert!(!is_safe_rel_path("mods/../../evil"));
        assert!(!is_safe_rel_path("/etc/passwd"));
        assert!(!is_safe_rel_path("C:/Windows"));
        assert!(!is_safe_rel_path("mods\\x.jar"));
        assert!(!is_safe_rel_path("mods//x.jar"));
        assert!(!is_safe_rel_path("./x"));
    }

    #[test]
    fn object_layout() {
        let h = "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567";
        assert!(is_sha1(h));
        assert_eq!(object_file(h), format!("objects/0a/{h}"));
    }
}
