mod catalog;
mod game;
mod local;
mod settings;

use scam_core::remote::{CachedStore, Store};
use scam_core::yadisk::{PublicDisk, http_client};
use serde::Serialize;
use settings::{Settings, SettingsStore};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

struct AppState {
    store: CachedStore,
    http: reqwest::Client,
    settings: SettingsStore,
    game: Arc<game::Game>,
}

impl AppState {
    fn ctx(&self, app: tauri::AppHandle) -> game::Ctx {
        game::Ctx {
            app,
            store: self.store.clone(),
            http: self.http.clone(),
            dirs: self.settings.dirs(),
            game: self.game.clone(),
        }
    }

    /// Запрос с настройками игрока для этой сборки.
    fn request(&self, pack: String, build: u64, task: game::Task) -> game::PlayRequest {
        let ps = self
            .settings
            .get()
            .packs
            .get(&pack)
            .cloned()
            .unwrap_or_default();
        game::PlayRequest {
            pack,
            build,
            task,
            optional: ps.optional,
            backup_worlds: ps.backup_worlds,
        }
    }

    fn public_disk(&self) -> PublicDisk {
        PublicDisk::new(
            self.http.clone(),
            scam_core::config::project().yandex.public_url.clone(),
        )
    }
}

type Res<T> = Result<T, String>;

#[tauri::command]
async fn get_catalog(state: tauri::State<'_, AppState>, beta: bool) -> Res<catalog::CatalogDto> {
    catalog::catalog(&state.store, &state.settings.dirs(), beta).await
}

#[tauri::command]
async fn get_build(
    state: tauri::State<'_, AppState>,
    pack: String,
    build: u64,
) -> Res<catalog::BuildDto> {
    catalog::build(&state.store, &state.settings.dirs(), &pack, build).await
}

#[tauri::command]
async fn play(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pack: String,
    build: u64,
    nick: String,
) -> Res<()> {
    let s = state.settings.get();
    let pack_settings = s.packs.get(&pack).cloned().unwrap_or_default();
    let mut jvm_args = settings::split_args(&s.jvm_args)?;
    jvm_args.extend(settings::split_args(&pack_settings.jvm_args)?);
    let own_server = Some(pack_settings.server.trim().to_owned()).filter(|s| !s.is_empty());
    let task = game::Task::Play {
        nick,
        memory_mb: pack_settings.memory_mb,
        jvm_args,
        auto_connect: pack_settings.auto_connect.then_some(own_server),
    };
    game::start(state.ctx(app), state.request(pack, build, task)).await
}

#[tauri::command]
async fn install(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pack: String,
    build: u64,
) -> Res<()> {
    let task = game::Task::Install;
    game::start(state.ctx(app), state.request(pack, build, task)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstanceInfo {
    /// Установленный билд; `None` — сборка ещё не скачана.
    installed_build: Option<u64>,
}

#[tauri::command]
fn instance_info(state: tauri::State<'_, AppState>, pack: String) -> Res<InstanceInfo> {
    if !scam_core::paths::is_valid_id(&pack) {
        return Err("Неверный id сборки".into());
    }
    let dir = state.settings.dirs().instance(&pack);
    Ok(InstanceInfo {
        installed_build: scam_core::sync::InstanceState::load(&dir).build,
    })
}

/// Удаляет папку сборки с компьютера — в корзину, чтобы можно было вернуть.
#[tauri::command]
async fn delete_instance(state: tauri::State<'_, AppState>, pack: String) -> Res<()> {
    if !scam_core::paths::is_valid_id(&pack) {
        return Err("Неверный id сборки".into());
    }
    if state.game.running_pack().await.is_some() {
        return Err("Сначала закрой игру".into());
    }
    let dir = state.settings.dirs().instance(&pack);
    if !dir.exists() {
        return Ok(());
    }
    tauri::async_runtime::spawn_blocking(move || trash::delete(&dir))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| {
            format!(
                "Не удалось переместить папку в корзину: {e}. Удали её вручную через «Папка игры»."
            )
        })
}

/// Открывает ссылку в браузере (только http/https).
#[tauri::command]
fn open_link(url: String) -> Res<()> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err("Открываются только ссылки http/https".into());
    }
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
async fn repair(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pack: String,
    build: u64,
) -> Res<()> {
    let task = game::Task::Repair;
    game::start(state.ctx(app), state.request(pack, build, task)).await
}

#[tauri::command]
async fn restore(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    pack: String,
    build: u64,
    groups: Vec<String>,
) -> Res<()> {
    if groups.is_empty() {
        return Err("Не выбрано ни одной группы".into());
    }
    let task = game::Task::Restore { groups };
    game::start(state.ctx(app), state.request(pack, build, task)).await
}

#[tauri::command]
async fn kill_game(state: tauri::State<'_, AppState>) -> Res<bool> {
    Ok(state.game.kill().await)
}

#[tauri::command]
async fn running_pack(state: tauri::State<'_, AppState>) -> Res<Option<String>> {
    Ok(state.game.running_pack().await)
}

/// Открывает папку игры сборки, её `mods-clients` или бэкапы миров.
#[tauri::command]
fn open_instance_dir(
    state: tauri::State<'_, AppState>,
    pack: String,
    what: Option<String>,
) -> Res<()> {
    if !scam_core::paths::is_valid_id(&pack) {
        return Err("Неверный id сборки".into());
    }
    let base = state.settings.dirs().instance(&pack);
    let dir = match what.as_deref() {
        None => base,
        Some("clientMods") => base.join(scam_core::sync::CLIENT_MODS_DIR),
        Some("worldBackups") => base
            .join(scam_core::sync::STATE_DIR)
            .join(scam_core::sync::WORLD_BACKUPS_DIR),
        Some(other) => return Err(format!("неизвестная папка {other}")),
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    tauri_plugin_opener::open_path(&dir, None::<&str>).map_err(|e| e.to_string())
}

/// Открывает файл внутри папки лаунчера (например, отчёт о сбое).
#[tauri::command]
fn open_game_file(state: tauri::State<'_, AppState>, path: String) -> Res<()> {
    let root = state
        .settings
        .dirs()
        .root
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let file = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !file.starts_with(&root) {
        return Err("Файл вне папки лаунчера".into());
    }
    tauri_plugin_opener::open_path(&file, None::<&str>).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsDto {
    settings: Settings,
    game_dir: String,
    default_game_dir: String,
    total_memory_mb: u64,
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> SettingsDto {
    SettingsDto {
        settings: state.settings.get(),
        game_dir: state.settings.dirs().root.to_string_lossy().into_owned(),
        default_game_dir: scam_mc::GameDirs::default_root()
            .to_string_lossy()
            .into_owned(),
        total_memory_mb: settings::total_memory_mb(),
    }
}

/// Сохраняет настройки. Папка игры меняется только через `move_game_dir`.
#[tauri::command]
fn save_settings(state: tauri::State<'_, AppState>, settings: Settings) -> Res<()> {
    settings::split_args(&settings.jvm_args)?;
    for p in settings.packs.values() {
        settings::split_args(&p.jvm_args)?;
    }
    let current = state.settings.get();
    state.settings.save(Settings {
        game_dir: current.game_dir,
        ..settings
    })
}

#[tauri::command]
async fn move_game_dir(
    state: tauri::State<'_, AppState>,
    path: String,
    move_files: bool,
) -> Res<()> {
    if state.game.running_pack().await.is_some() {
        return Err("Сначала закрой игру".into());
    }
    let new = PathBuf::from(&path);
    if !new.is_absolute() {
        return Err("Нужен полный путь к папке".into());
    }
    let old = state.settings.dirs().root;
    if new == old {
        return Ok(());
    }
    if new.starts_with(&old) || old.starts_with(&new) {
        return Err("Новая папка не может быть внутри старой (и наоборот)".into());
    }
    if move_files && old.exists() {
        let (from, to) = (old.clone(), new.clone());
        tauri::async_runtime::spawn_blocking(move || settings::move_dir(&from, &to))
            .await
            .map_err(|e| e.to_string())??;
    }
    let mut s = state.settings.get();
    s.game_dir = (new != scam_mc::GameDirs::default_root()).then_some(new);
    state.settings.save(s)
}

/// Статус сервера для карточки сборки.
#[tauri::command]
async fn server_status(address: String) -> Res<scam_mc::server::Status> {
    let addr = scam_core::server::ServerAddress::parse(&address)?;
    scam_mc::server::ping(&addr)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn total_memory_mb() -> u64 {
    settings::total_memory_mb()
}

/// Картинка сборки (иконка, фон) как data URL; кэшируется в папке игры.
#[tauri::command]
async fn get_image(state: tauri::State<'_, AppState>, sha1: String) -> Res<String> {
    use base64::Engine;
    if !scam_core::paths::is_sha1(&sha1) {
        return Err("неверный sha1".into());
    }
    let cache = state.settings.dirs().root.join("cache").join("images");
    let path = cache.join(&sha1);
    if !path.is_file() {
        std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let tmp = cache.join(format!("{sha1}.part"));
        let (got, _) = state
            .public_disk()
            .download_to(&scam_core::paths::object_file(&sha1), &tmp, &|_| {})
            .await
            .map_err(|e| e.to_string())?;
        if got != sha1 {
            let _ = std::fs::remove_file(&tmp);
            return Err("картинка повреждена".into());
        }
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mime = image_mime(&bytes).ok_or("неподдерживаемый формат картинки")?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn image_mime(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG") {
        Some("image/png")
    } else if b.starts_with(b"\xFF\xD8\xFF") {
        Some("image/jpeg")
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else if b.starts_with(b"GIF8") {
        Some("image/gif")
    } else {
        None
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let http = http_client(concat!("ScamLauncher/", env!("CARGO_PKG_VERSION")));
    // Каталог: две попытки — без сети быстро переходим на сохранённую копию.
    let public = PublicDisk::new(
        http.clone(),
        scam_core::config::project().yandex.public_url.clone(),
    )
    .with_attempts(2);

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let path = app.path().app_config_dir()?.join("settings.json");
            let cache = app.path().app_cache_dir()?.join("catalog");
            app.manage(AppState {
                store: CachedStore::new(Store::Public(public), cache),
                http,
                settings: SettingsStore::load(path),
                game: Arc::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_catalog,
            get_build,
            play,
            install,
            instance_info,
            delete_instance,
            open_link,
            repair,
            restore,
            kill_game,
            running_pack,
            open_instance_dir,
            open_game_file,
            get_settings,
            save_settings,
            move_game_dir,
            total_memory_mb,
            server_status,
            get_image
        ])
        .run(tauri::generate_context!())
        .expect("не удалось запустить ScamLauncher");
}

#[cfg(test)]
mod tests {
    #[test]
    fn mimes() {
        assert_eq!(super::image_mime(b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(super::image_mime(b"\xFF\xD8\xFF\xE0"), Some("image/jpeg"));
        assert_eq!(
            super::image_mime(b"RIFF\0\0\0\0WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(super::image_mime(b"<svg"), None);
    }
}
