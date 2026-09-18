//! Встроенный конфиг проекта (`scam.config.json` в корне репозитория).

use serde::Deserialize;
use std::sync::OnceLock;

const RAW: &str = include_str!("../../../scam.config.json");

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectConfig {
    pub yandex: YandexConfig,
    pub github: GithubConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct YandexConfig {
    /// Публичная ссылка на корневую папку со сборками.
    pub public_url: String,
    /// ClientID OAuth-приложения Яндекса (нужен только `scam-pack`).
    pub oauth_client_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubConfig {
    /// `owner/repo`
    pub repo: String,
}

pub fn project() -> &'static ProjectConfig {
    static CONFIG: OnceLock<ProjectConfig> = OnceLock::new();
    CONFIG.get_or_init(|| serde_json::from_str(RAW).expect("scam.config.json повреждён"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_config_parses() {
        let cfg = super::project();
        assert!(cfg.yandex.public_url.starts_with("https://"));
        assert!(cfg.github.repo.contains('/'));
    }
}
