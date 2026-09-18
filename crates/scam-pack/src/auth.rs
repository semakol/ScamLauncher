//! OAuth-токен Яндекса для записи на диск.

use crate::ui;
use anyhow::{Context, Result, bail};
use console::style;
use scam_core::yadisk::Disk;
use std::io::{BufRead, Write};
use std::path::PathBuf;

const ENV_TOKEN: &str = "SCAM_YANDEX_TOKEN";

fn token_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("не найдена папка настроек пользователя")?;
    Ok(dir.join("scam-pack").join("token"))
}

/// Токен из `SCAM_YANDEX_TOKEN` или сохранённый `scam-pack login`.
pub fn load_token() -> Result<Option<String>> {
    if let Ok(t) = std::env::var(ENV_TOKEN)
        && !t.trim().is_empty()
    {
        return Ok(Some(t.trim().to_owned()));
    }
    let path = token_path()?;
    match std::fs::read_to_string(&path) {
        Ok(t) if !t.trim().is_empty() => Ok(Some(t.trim().to_owned())),
        Ok(_) => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("не удалось прочитать {}", path.display())),
    }
}

pub fn require_token() -> Result<String> {
    load_token()?.context("нет токена Яндекса — сначала выполни `scam-pack login`")
}

fn save_token(token: &str) -> Result<PathBuf> {
    let path = token_path()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        f.write_all(token.as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&path, token)?;
    Ok(path)
}

pub fn logout() -> Result<()> {
    let path = token_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => ui::ok("Токен удалён"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ui::ok("Токен и так не сохранён"),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

/// Достаёт токен из вставленного текста: сам токен или адрес страницы с `#access_token=…`.
fn extract_token(input: &str) -> Option<String> {
    let input = input.trim();
    if let Some((_, tail)) = input.split_once("access_token=") {
        let token = tail.split(['&', '#', ' ']).next()?;
        return (!token.is_empty()).then(|| token.to_owned());
    }
    (!input.is_empty() && !input.contains(char::is_whitespace)).then(|| input.to_owned())
}

pub async fn login(http: reqwest::Client, token: Option<String>) -> Result<()> {
    let token = match token {
        Some(t) => t,
        None => {
            let client_id = &scam_core::config::project().yandex.oauth_client_id;
            let url = format!(
                "https://oauth.yandex.ru/authorize?response_type=token&client_id={client_id}&force_confirm=yes"
            );
            println!(
                "Открой страницу, войди в аккаунт Яндекса, где лежит папка сборок, и нажми «Разрешить»:"
            );
            println!("  {}", style(&url).underlined());
            if open::that(&url).is_err() {
                ui::warn("Браузер не открылся — скопируй ссылку вручную");
            }
            print!("Вставь токен со страницы: ");
            std::io::stdout().flush()?;
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            line
        }
    };
    let Some(token) = extract_token(&token) else {
        bail!("пустой токен");
    };

    let disk = Disk::new(http, token.clone());
    let info = disk.info().await.context("Яндекс не принял токен")?;
    let root = crate::publisher::resolve_root(&disk).await?;
    let path = save_token(&token)?;

    let who = info
        .user
        .map(|u| {
            if u.display_name.is_empty() {
                u.login
            } else {
                format!("{} ({})", u.display_name, u.login)
            }
        })
        .unwrap_or_default();
    ui::ok(format!("Вход выполнен: {who}"));
    ui::ok(format!(
        "Папка сборок: {root}, свободно {}",
        ui::size(info.total_space.saturating_sub(info.used_space))
    ));
    println!("  Токен сохранён в {}", style(path.display()).dim());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::extract_token;

    #[test]
    fn tokens() {
        assert_eq!(
            extract_token("  y0_AgAAAA \n").as_deref(),
            Some("y0_AgAAAA")
        );
        assert_eq!(
            extract_token("https://oauth.yandex.ru/verification_code#access_token=y0_abc&token_type=bearer&expires_in=31536000").as_deref(),
            Some("y0_abc")
        );
        assert_eq!(extract_token("   "), None);
        assert_eq!(extract_token("два слова"), None);
    }
}
