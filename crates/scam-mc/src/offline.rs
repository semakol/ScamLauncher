use md5::{Digest, Md5};

/// UUID оффлайн-игрока как у ванильного сервера: `UUID.nameUUIDFromBytes("OfflinePlayer:" + ник)`.
/// Без дефисов — в таком виде его принимает `--uuid`.
pub fn offline_uuid(name: &str) -> String {
    let mut bytes: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    hex::encode(bytes)
}

/// Ник для оффлайн-входа: 3–16 символов, латиница, цифры, `_`.
pub fn is_valid_nick(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    #[test]
    fn notch() {
        assert_eq!(
            super::offline_uuid("Notch"),
            "b50ad385829d3141a2167e7d7539ba7f"
        );
    }

    #[test]
    fn nicks() {
        assert!(super::is_valid_nick("Steve_2"));
        assert!(!super::is_valid_nick("ab"));
        assert!(!super::is_valid_nick("Стив"));
        assert!(!super::is_valid_nick("a b c"));
        assert!(!super::is_valid_nick("seventeen_chars_x"));
    }
}
