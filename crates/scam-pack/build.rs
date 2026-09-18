fn main() {
    // Целевая платформа — чтобы self-update знал, какой файл брать из релиза.
    println!(
        "cargo:rustc-env=SCAM_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
