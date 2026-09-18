//! Кнопка «Играть»: установка версии, запуск, логи, завершение.

use scam_core::remote::CachedStore;
use scam_core::sync::{self, PublicObjects, SyncOptions, SyncProgress, SyncReport};
use scam_mc::launch::{self, LaunchOptions};
use scam_mc::log::LogLine;
use scam_mc::{GameDirs, Mirrors, Net, Progress, install};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, oneshot};

const EVENT: &str = "game";

/// События для интерфейса (одно имя события, тип — в поле `kind`).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GameEvent {
    Stage {
        pack: String,
        text: String,
    },
    Bytes {
        pack: String,
        done: u64,
        total: u64,
    },
    /// Файлы сборки приведены в порядок перед запуском.
    Synced {
        pack: String,
        report: SyncReport,
    },
    /// Починка или восстановление закончены (без запуска игры).
    Done {
        pack: String,
        task: &'static str,
        report: SyncReport,
    },
    Started {
        pack: String,
    },
    Log {
        pack: String,
        lines: Vec<LogLine>,
    },
    #[serde(rename_all = "camelCase")]
    Exited {
        pack: String,
        code: Option<i32>,
        crashed: bool,
        killed: bool,
        crash_report: Option<String>,
    },
    Failed {
        pack: String,
        message: String,
    },
}

struct Running {
    pack: String,
    kill: Option<oneshot::Sender<()>>,
}

#[derive(Default)]
pub struct Game {
    running: Mutex<Option<Running>>,
}

pub enum Task {
    Play {
        nick: String,
        /// Память из настроек сборки; `None` — рекомендованная сборкой.
        memory_mb: Option<u32>,
        jvm_args: Vec<String>,
        /// Автоподключение: свой адрес игрока или `None` — адрес из сборки.
        auto_connect: Option<Option<String>>,
    },
    /// «Установить» / «Обновить»: скачать всё, но игру не запускать.
    Install,
    /// «Проверить и починить»: всё с пересчётом хэшей.
    Repair,
    /// «Восстановить файлы сборки»: заново поставить выбранные once-группы.
    Restore { groups: Vec<String> },
}

pub struct PlayRequest {
    pub pack: String,
    pub build: u64,
    pub task: Task,
    /// Выбор опциональных модов: id группы → включена.
    pub optional: std::collections::BTreeMap<String, bool>,
    pub backup_worlds: bool,
}

/// Сколько бэкапов миров хранить.
const WORLD_BACKUPS: usize = 3;

pub struct Ctx {
    pub app: AppHandle,
    pub store: CachedStore,
    pub http: reqwest::Client,
    pub dirs: GameDirs,
    pub game: Arc<Game>,
}

fn emit(app: &AppHandle, event: GameEvent) {
    let _ = app.emit(EVENT, event);
}

impl Game {
    pub async fn running_pack(&self) -> Option<String> {
        self.running.lock().await.as_ref().map(|r| r.pack.clone())
    }

    pub async fn kill(&self) -> bool {
        match self
            .running
            .lock()
            .await
            .as_mut()
            .and_then(|r| r.kill.take())
        {
            Some(tx) => tx.send(()).is_ok(),
            None => false,
        }
    }
}

/// Запускает установку и игру в фоне. Ошибка — только если запустить нельзя сразу.
pub async fn start(ctx: Ctx, req: PlayRequest) -> Result<(), String> {
    if let Task::Play { nick, .. } = &req.task
        && !scam_mc::offline::is_valid_nick(nick)
    {
        return Err("Ник: 3–16 символов, латиница, цифры и _".into());
    }
    if !scam_core::paths::is_valid_id(&req.pack) {
        return Err("Неверный id сборки".into());
    }
    {
        let mut running = ctx.game.running.lock().await;
        if running.is_some() {
            return Err("Игра уже запущена или идёт подготовка".into());
        }
        *running = Some(Running {
            pack: req.pack.clone(),
            kill: None,
        });
    }

    tauri::async_runtime::spawn(async move {
        let pack = req.pack.clone();
        if let Err(e) = run(&ctx, &req).await {
            emit(
                &ctx.app,
                GameEvent::Failed {
                    pack,
                    message: friendly_error(&e),
                },
            );
        }
        ctx.game.running.lock().await.take();
    });
    Ok(())
}

fn friendly_error(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    if text.contains("os error 86") || text.contains("Bad CPU type") {
        return "Для этой версии Minecraft на Mac с чипом Apple нужен Rosetta. \
                Установи его командой в Терминале: softwareupdate --install-rosetta"
            .into();
    }
    text
}

async fn run(ctx: &Ctx, req: &PlayRequest) -> anyhow::Result<()> {
    let app = &ctx.app;
    let pack = req.pack.clone();
    emit(
        app,
        GameEvent::Stage {
            pack: pack.clone(),
            text: "Загрузка сборки".into(),
        },
    );
    let game_dir = ctx.dirs.instance(&req.pack);
    let index = ctx.store.index().await?;
    // Удалена с сервера: её нет ни среди видимых, ни среди скрытых сборок.
    let removed = !index.offline && index.value.pack(&req.pack).is_none();
    let manifest = match ctx.store.build(&req.pack, req.build).await {
        Ok(m) => m,
        // Билда на сервере уже нет — берём копию, сохранённую при установке.
        Err(e) => match crate::local::load_manifest(&game_dir) {
            Some(m) if m.build == req.build => m,
            _ => return Err(e.into()),
        },
    };
    if removed && !matches!(req.task, Task::Play { .. }) {
        anyhow::bail!(
            "Сборку удалили с сервера: обновить, починить или восстановить её нельзя. Играть можно как есть."
        );
    }
    if removed {
        emit(
            app,
            GameEvent::Stage {
                pack: pack.clone(),
                text: "Сборку удалили с сервера — запускаю как есть".into(),
            },
        );
    }
    if index.offline {
        emit(
            app,
            GameEvent::Stage {
                pack: pack.clone(),
                text: "Нет связи с сервером — запускаю сохранённую версию".into(),
            },
        );
    }
    let net = Net::new(ctx.http.clone(), Mirrors::from_config(&index.value.mirrors));

    let last_bytes = StdMutex::new(Instant::now() - Duration::from_secs(1));
    let progress = |p: Progress| match p {
        Progress::Stage(text) => emit(
            app,
            GameEvent::Stage {
                pack: pack.clone(),
                text,
            },
        ),
        Progress::Bytes { done, total } => {
            let mut last = last_bytes.lock().unwrap();
            if last.elapsed() >= Duration::from_millis(100) || done >= total {
                *last = Instant::now();
                emit(
                    app,
                    GameEvent::Bytes {
                        pack: pack.clone(),
                        done,
                        total,
                    },
                );
            }
        }
    };
    let target = install::Target {
        minecraft: &manifest.minecraft,
        loader: manifest.loader.kind,
        loader_version: manifest.loader.version.as_deref(),
    };
    let verify = matches!(req.task, Task::Repair);
    let prepared = match req.task {
        Task::Restore { .. } => None,
        _ => Some(
            install::prepare(
                &net,
                &ctx.dirs,
                &target,
                &install::Options {
                    verify,
                    java_major: manifest.java,
                },
                &progress,
            )
            .await?,
        ),
    };

    let sync_progress = |p: SyncProgress| match p {
        SyncProgress::Stage(text) => progress(Progress::Stage(text)),
        SyncProgress::Bytes { done, total } => progress(Progress::Bytes { done, total }),
    };
    let source = PublicObjects(scam_core::yadisk::PublicDisk::new(
        ctx.http.clone(),
        scam_core::config::project().yandex.public_url.clone(),
    ));
    let reinstall = match &req.task {
        Task::Restore { groups } => groups.clone(),
        _ => Vec::new(),
    };
    let report = if removed {
        // Скачивать неоткуда — запускаем то, что уже есть на диске.
        SyncReport::default()
    } else {
        let report = sync::sync(
            &game_dir,
            &manifest,
            &source,
            &SyncOptions {
                verify,
                reinstall,
                disabled: manifest
                    .groups
                    .iter()
                    .filter(|g| {
                        g.optional
                            && !req
                                .optional
                                .get(&g.id)
                                .copied()
                                .unwrap_or(g.enabled_by_default)
                    })
                    .map(|g| g.id.clone())
                    .collect(),
                world_backups: if req.backup_worlds { WORLD_BACKUPS } else { 0 },
            },
            &sync_progress,
        )
        .await?;
        if let Some(entry) = index.value.pack(&req.pack) {
            crate::local::save(&game_dir, entry, &manifest);
        }
        report
    };

    let (nick, memory_mb, jvm_args, auto_connect, prepared) = match (&req.task, prepared) {
        (
            Task::Play {
                nick,
                memory_mb,
                jvm_args,
                auto_connect,
            },
            Some(p),
        ) => (
            nick.clone(),
            *memory_mb,
            jvm_args.clone(),
            auto_connect.clone(),
            p,
        ),
        (task, _) => {
            let task = match task {
                Task::Install => "install",
                Task::Repair => "repair",
                _ => "restore",
            };
            emit(app, GameEvent::Done { pack, task, report });
            return Ok(());
        }
    };
    emit(
        app,
        GameEvent::Synced {
            pack: pack.clone(),
            report,
        },
    );

    // Автоподключение: адрес игрока или из сборки; SRV ищем здесь для старых версий.
    let server = match auto_connect {
        Some(own) => {
            let address =
                own.or_else(|| index.value.pack(&req.pack).and_then(|p| p.server.clone()));
            match address.map(|a| scam_core::server::ServerAddress::parse(&a)) {
                Some(Ok(addr)) => {
                    let (host, port) = scam_mc::server::resolve(&addr).await;
                    Some(launch::ServerTarget {
                        address: addr.to_string(),
                        host,
                        port,
                    })
                }
                Some(Err(e)) => anyhow::bail!("адрес сервера: {e}"),
                None => None,
            }
        }
        None => None,
    };
    let max_memory = memory_mb
        .or(manifest.memory.recommended)
        .unwrap_or(4096)
        .max(512);
    let opts = LaunchOptions {
        game_dir: game_dir.clone(),
        player: nick,
        memory_max_mb: max_memory,
        memory_min_mb: manifest.memory.min.filter(|min| *min <= max_memory),
        extra_jvm_args: jvm_args,
        launcher_name: "ScamLauncher".into(),
        launcher_version: app.package_info().version.to_string(),
        server,
    };
    emit(
        app,
        GameEvent::Stage {
            pack: pack.clone(),
            text: "Запуск игры".into(),
        },
    );
    let started_at = SystemTime::now();
    let mut child = launch::launch(&ctx.dirs, &prepared, &opts)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("нет stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("нет stderr"))?;

    let (kill_tx, kill_rx) = oneshot::channel();
    if let Some(r) = ctx.game.running.lock().await.as_mut() {
        r.kill = Some(kill_tx);
    }
    emit(app, GameEvent::Started { pack: pack.clone() });

    // Логи копятся и уходят в интерфейс пачками.
    let buffer: Arc<StdMutex<Vec<LogLine>>> = Arc::default();
    let flush = {
        let buffer = buffer.clone();
        let pack = pack.clone();
        move || {
            let lines = std::mem::take(&mut *buffer.lock().unwrap());
            if !lines.is_empty() {
                emit(
                    app,
                    GameEvent::Log {
                        pack: pack.clone(),
                        lines,
                    },
                );
            }
        }
    };
    let mut log_file = open_log_file(&game_dir);
    let logs = {
        let buffer = buffer.clone();
        launch::read_logs(stdout, stderr, move |l| {
            if let Some(f) = log_file.as_mut() {
                use std::io::Write;
                let prefix = l
                    .level
                    .as_deref()
                    .map(|lv| format!("[{lv}] "))
                    .unwrap_or_default();
                let _ = writeln!(f, "{prefix}{}", l.text);
            }
            buffer.lock().unwrap().push(l)
        })
    };
    let mut killed = false;
    let wait = async {
        tokio::select! {
            status = child.wait() => status,
            _ = kill_rx => {
                killed = true;
                let _ = child.kill().await;
                child.wait().await
            }
        }
    };
    let ticker = async {
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            flush();
        }
    };
    let (logs_res, status) = tokio::select! {
        r = async { tokio::join!(logs, wait) } => r,
        _ = ticker => unreachable!(),
    };
    flush();
    if let Err(e) = logs_res {
        eprintln!("ошибка чтения логов: {e:#}");
    }
    let status = status?;

    let code = status.code();
    let crashed = !killed && !status.success();
    let crash_report = if crashed {
        newest_crash_report(&game_dir, started_at).map(|p| p.to_string_lossy().into_owned())
    } else {
        None
    };
    emit(
        app,
        GameEvent::Exited {
            pack,
            code,
            crashed,
            killed,
            crash_report,
        },
    );
    Ok(())
}

/// `.scam/logs/latest.log` (прошлый запуск — в `previous.log`).
fn open_log_file(game_dir: &Path) -> Option<std::io::BufWriter<std::fs::File>> {
    let dir = game_dir.join(scam_core::sync::STATE_DIR).join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    let latest = dir.join("latest.log");
    let _ = std::fs::rename(&latest, dir.join("previous.log"));
    std::fs::File::create(latest)
        .ok()
        .map(std::io::BufWriter::new)
}

/// Самый свежий отчёт о сбое, созданный после запуска.
fn newest_crash_report(game_dir: &Path, since: SystemTime) -> Option<PathBuf> {
    std::fs::read_dir(game_dir.join("crash-reports"))
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            (modified >= since).then(|| (modified, e.path()))
        })
        .max_by_key(|(m, _)| *m)
        .map(|(_, p)| p)
}
