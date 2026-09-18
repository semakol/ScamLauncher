use sha1::{Digest, Sha1};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// SHA-1 и размер файла.
pub fn sha1_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), size))
}

pub fn sha1_bytes(data: &[u8]) -> String {
    hex::encode(Sha1::digest(data))
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_hash() {
        assert_eq!(
            super::sha1_bytes(b"abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"abc").unwrap();
        let (h, size) = super::sha1_file(&p).unwrap();
        assert_eq!(h, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(size, 3);
    }
}
