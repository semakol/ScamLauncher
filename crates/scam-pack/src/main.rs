mod auth;
mod commands;
mod packfile;
mod publisher;
mod scan;
mod ui;

use clap::{Parser, Subcommand};
use console::style;
use scam_core::model::LoaderKind;
use std::path::PathBuf;
use std::process::ExitCode;

/// Публикация сборок ScamLauncher на Яндекс Диск.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Путь к pack.toml
    #[arg(short, long, global = true, default_value = packfile::FILE_NAME)]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Войти в Яндекс: откроется страница, токен вставить в консоль
    Login {
        /// Готовый OAuth-токен (например, с Полигона Яндекса)
        #[arg(long)]
        token: Option<String>,
    },
    /// Удалить сохранённый токен
    Logout,
    /// Создать pack.toml в текущей папке
    Init {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "1.20.1")]
        minecraft: String,
        /// vanilla | fabric | quilt | forge | neoforge
        #[arg(long, default_value = "fabric")]
        loader: LoaderKind,
        #[arg(long)]
        loader_version: Option<String>,
        /// Папка с файлами сборки относительно pack.toml
        #[arg(long, default_value = ".")]
        source: String,
        /// Перезаписать существующий pack.toml
        #[arg(long)]
        force: bool,
    },
    /// Что изменится при публикации (ничего не заливает, токен не нужен)
    Status {
        /// Сравнивать с бетой
        #[arg(long)]
        beta: bool,
    },
    /// Опубликовать сборку
    Publish {
        /// В бета-канал
        #[arg(long)]
        beta: bool,
        /// Отображаемая версия, например 1.3.0 (по умолчанию — как у прошлого билда)
        #[arg(long)]
        version: Option<String>,
        /// Что нового (текст)
        #[arg(short, long)]
        message: Option<String>,
        /// Что нового (из файла, Markdown)
        #[arg(long)]
        changelog: Option<PathBuf>,
        /// Выпустить билд, даже если ничего не изменилось
        #[arg(long)]
        force: bool,
        /// Пропускать файлы, не попавшие ни в одну группу
        #[arg(long)]
        allow_unmatched: bool,
    },
    /// Сделать билд релизом: бету (по умолчанию) или любой старый (откат)
    Promote {
        pack: String,
        #[arg(long)]
        build: Option<u64>,
    },
    /// Список сборок на диске
    List,
    /// Новости в лаунчере
    News {
        #[command(subcommand)]
        action: NewsAction,
    },
    /// Удалить старые билды и неиспользуемые файлы
    Gc {
        /// Сколько последних билдов каждой сборки оставить (текущие релиз и бета — всегда)
        #[arg(long)]
        keep: Option<usize>,
        /// Удалять мимо корзины
        #[arg(long)]
        permanently: bool,
        /// Действительно удалить (без флага — пробный прогон)
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum NewsAction {
    /// Добавить новость
    Add {
        #[arg(long)]
        title: String,
        /// Привязать к сборке
        #[arg(long)]
        pack: Option<String>,
        text: String,
    },
    /// Показать новости
    List,
    /// Удалить новость по id
    Rm { id: String },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{} {e:#}", style("Ошибка:").red().bold());
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let http = scam_core::yadisk::http_client(concat!("scam-pack/", env!("CARGO_PKG_VERSION")));
    let config = cli.config;
    match cli.command {
        Command::Login { token } => auth::login(http, token).await,
        Command::Logout => auth::logout(),
        Command::Init {
            id,
            name,
            minecraft,
            loader,
            loader_version,
            source,
            force,
        } => commands::init(
            &config,
            commands::InitArgs {
                id,
                name,
                minecraft,
                loader,
                loader_version,
                source,
                force,
            },
        ),
        Command::Status { beta } => commands::status(http, &config, beta).await,
        Command::Publish {
            beta,
            version,
            message,
            changelog,
            force,
            allow_unmatched,
        } => {
            commands::publish(
                http,
                &config,
                commands::PublishArgs {
                    beta,
                    version,
                    message,
                    changelog,
                    force,
                    allow_unmatched,
                },
            )
            .await
        }
        Command::Promote { pack, build } => commands::promote(http, &pack, build).await,
        Command::List => commands::list(http).await,
        Command::News { action } => match action {
            NewsAction::Add { title, pack, text } => {
                commands::news_add(http, title, text, pack).await
            }
            NewsAction::List => commands::news_list(http).await,
            NewsAction::Rm { id } => commands::news_rm(http, &id).await,
        },
        Command::Gc {
            keep,
            permanently,
            yes,
        } => commands::gc(http, keep, permanently, yes).await,
    }
}
