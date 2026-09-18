mod catalog;

use scam_core::remote::Store;
use scam_core::yadisk::{PublicDisk, http_client};

struct AppState {
    store: Store,
}

#[tauri::command]
async fn get_catalog(
    state: tauri::State<'_, AppState>,
    beta: bool,
) -> Result<catalog::CatalogDto, String> {
    catalog::catalog(&state.store, beta).await
}

#[tauri::command]
async fn get_build(
    state: tauri::State<'_, AppState>,
    pack: String,
    build: u64,
) -> Result<catalog::BuildDto, String> {
    catalog::build(&state.store, &pack, build).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let http = http_client(concat!("ScamLauncher/", env!("CARGO_PKG_VERSION")));
    let store = Store::Public(PublicDisk::new(
        http,
        scam_core::config::project().yandex.public_url.clone(),
    ));

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState { store })
        .invoke_handler(tauri::generate_handler![get_catalog, get_build])
        .run(tauri::generate_context!())
        .expect("не удалось запустить ScamLauncher");
}
