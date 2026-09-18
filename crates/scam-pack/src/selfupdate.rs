//! `scam-pack self-update`: свежая версия из GitHub Releases.

use crate::ui;
use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use reqwest::header::ACCEPT;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
    /// `sha256:<hex>` — GitHub считает сам.
    #[serde(default)]
    digest: Option<String>,
}

/// Имя файла в релизе для этой платформы, как его выкладывает release.yml.
fn asset_name() -> String {
    format!(
        "scam-pack-{}{}",
        env!("SCAM_TARGET"),
        std::env::consts::EXE_SUFFIX
    )
}

fn parse_version(tag: &str) -> Result<Version> {
    Version::parse(tag.trim_start_matches('v'))
        .with_context(|| format!("странный тег релиза «{tag}»"))
}

async fn latest_release(http: &reqwest::Client) -> Result<Option<Release>> {
    let repo = &scam_core::config::project().github.repo;
    let resp = http
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .context("не удалось связаться с GitHub")?;
    match resp.status() {
        StatusCode::NOT_FOUND => Ok(None),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
            bail!("GitHub временно ограничил запросы — попробуй через час")
        }
        _ => Ok(Some(resp.error_for_status()?.json().await?)),
    }
}

/// Скачивает файл релиза в `dest`, проверяя размер и sha256.
async fn download_verified(
    http: &reqwest::Client,
    asset: &Asset,
    dest: &mut impl Write,
) -> Result<()> {
    let mut resp = http
        .get(&asset.browser_download_url)
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("не удалось скачать {}", asset.name))?;
    let bar = ui::bytes_bar(asset.size, "Загрузка");
    let mut hasher = Sha256::new();
    let mut got = 0u64;
    while let Some(chunk) = resp.chunk().await? {
        hasher.update(&chunk);
        dest.write_all(&chunk)?;
        got += chunk.len() as u64;
        bar.set_position(got);
    }
    bar.finish_and_clear();
    dest.flush()?;

    if got != asset.size {
        bail!("файл скачался не полностью ({got} из {} байт)", asset.size);
    }
    if let Some(expected) = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
    {
        let actual = hex::encode(hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            bail!("контрольная сумма не совпала — файл повреждён, обновление отменено");
        }
    }
    Ok(())
}

pub async fn run(http: reqwest::Client, check_only: bool, force: bool) -> Result<()> {
    let current = parse_version(env!("CARGO_PKG_VERSION"))?;
    let Some(release) = latest_release(&http).await? else {
        ui::ok(format!("Релизов пока нет, у тебя {current}"));
        return Ok(());
    };
    let latest = parse_version(&release.tag_name)?;

    if latest <= current && !force {
        ui::ok(format!("Уже последняя версия: {current}"));
        return Ok(());
    }
    if check_only {
        if latest > current {
            ui::step(format!(
                "Доступна {latest} (у тебя {current}). Обновить: scam-pack self-update"
            ));
        } else {
            ui::ok(format!("Уже последняя версия: {current}"));
        }
        return Ok(());
    }

    let name = asset_name();
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .with_context(|| format!("в релизе {latest} нет файла {name} для этой системы"))?;

    // Временный файл рядом с текущим: так замена — это переименование в пределах одного диска.
    let exe = std::env::current_exe()?;
    let dir = exe.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::Builder::new()
        .prefix(".scam-pack-update")
        .tempfile_in(dir)
        .with_context(|| {
            format!(
                "нет прав на запись в {} — запусти от имени владельца этой папки",
                dir.display()
            )
        })?;
    ui::step(format!("Скачиваю {latest}…"));
    download_verified(&http, asset, tmp.as_file_mut()).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755))?;
    }
    self_replace::self_replace(tmp.path()).context("не удалось заменить исполняемый файл")?;
    ui::ok(format!("scam-pack обновлён: {current} → {latest}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_for_this_platform() {
        let name = asset_name();
        assert!(name.starts_with("scam-pack-"));
        assert!(name.contains(std::env::consts::ARCH) || name.contains("aarch64"));
    }

    #[test]
    fn versions() {
        assert!(parse_version("v0.2.0").unwrap() > parse_version("0.1.9").unwrap());
        assert!(parse_version("v1.0.0").unwrap() > parse_version("1.0.0-beta.1").unwrap());
        assert!(parse_version("latest").is_err());
    }

    /// Живая проверка загрузки и sha256 на чужом публичном релизе.
    /// `cargo test -p scam-pack -- --ignored download_checks_digest`
    #[tokio::test]
    #[ignore]
    async fn download_checks_digest() {
        let http = scam_core::yadisk::http_client("scam-pack-test");
        let rel: Release = http
            .get("https://api.github.com/repos/cli/cli/releases/latest")
            .header(ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let asset = rel
            .assets
            .iter()
            .find(|a| a.name.ends_with("checksums.txt"))
            .unwrap();
        assert!(asset.digest.is_some());

        let mut buf = Vec::new();
        download_verified(&http, asset, &mut buf).await.unwrap();
        assert_eq!(buf.len() as u64, asset.size);

        let broken = Asset {
            name: asset.name.clone(),
            size: asset.size,
            browser_download_url: asset.browser_download_url.clone(),
            digest: Some(format!("sha256:{}", "0".repeat(64))),
        };
        let err = download_verified(&http, &broken, &mut Vec::new())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("контрольная сумма"));
    }
}
