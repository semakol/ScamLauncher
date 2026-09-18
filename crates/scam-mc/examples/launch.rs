//! Ручная проверка установки и запуска:
//! `cargo run -p scam-mc --example launch -- 1.20.1 [--loader forge:47.4.0] [--nick Steve] [--dir папка] [--no-launch]`

use scam_core::model::LoaderKind;
use scam_mc::install::{self, Target};
use scam_mc::launch::{LaunchOptions, launch, pump_logs};
use scam_mc::{GameDirs, Mirrors, Net, Progress};
use std::io::Write;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let version = args.next().unwrap_or_else(|| "1.20.1".into());
    let (mut loader, mut player, mut dir, mut no_launch) = (None, "Steve".to_string(), None, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--loader" => loader = args.next(),
            "--nick" => player = args.next().unwrap(),
            "--dir" => dir = args.next(),
            "--no-launch" => no_launch = true,
            other => anyhow::bail!("неизвестный аргумент {other}"),
        }
    }
    let (kind, loader_version) = match &loader {
        Some(l) => {
            let (k, v) = l.split_once(':').expect("--loader вид:версия");
            (
                k.parse::<LoaderKind>().map_err(anyhow::Error::msg)?,
                Some(v),
            )
        }
        None => (LoaderKind::Vanilla, None),
    };
    let dirs = GameDirs::new(dir.map(Into::into).unwrap_or_else(GameDirs::default_root));

    let net = Net::new(
        scam_core::yadisk::http_client("ScamLauncher-dev"),
        Mirrors::default(),
    );
    let last = std::sync::Mutex::new(std::time::Instant::now());
    let progress = |p: Progress| match p {
        Progress::Stage(s) => println!("\n== {s}"),
        Progress::Bytes { done, total } => {
            let mut l = last.lock().unwrap();
            if l.elapsed().as_millis() > 500 || done == total {
                *l = std::time::Instant::now();
                print!(
                    "\r   {:.1} / {:.1} МБ",
                    done as f64 / 1e6,
                    total as f64 / 1e6
                );
                let _ = std::io::stdout().flush();
            }
        }
    };
    let started = std::time::Instant::now();
    let target = Target {
        minecraft: &version,
        loader: kind,
        loader_version,
    };
    let prepared = install::prepare(&net, &dirs, &target, &Default::default(), &progress).await?;
    println!(
        "\nГотово за {:.1} c: версия {}, java {}, arch {}, {} jar в classpath",
        started.elapsed().as_secs_f32(),
        prepared.version.id,
        prepared.java.display(),
        prepared.env.arch,
        prepared.classpath.len()
    );
    let opts = LaunchOptions {
        game_dir: dirs.instance(&format!("dev-{}", prepared.version.id)),
        player,
        memory_max_mb: 3072,
        memory_min_mb: None,
        extra_jvm_args: vec![],
        launcher_name: "ScamLauncher".into(),
        launcher_version: "dev".into(),
        server: None,
    };
    let cmd = scam_mc::launch::command_line(&dirs, &prepared, &opts)?;
    let missing: Vec<_> = prepared.classpath.iter().filter(|p| !p.is_file()).collect();
    println!("Классов в classpath нет на диске: {}", missing.len());
    for m in &missing {
        println!("   нет: {}", m.display());
    }
    if no_launch {
        println!("Аргументов запуска: {}", cmd.len());
        return Ok(());
    }

    let mut child = launch(&dirs, &prepared, &opts)?;
    let status = pump_logs(&mut child, |l| {
        let level = l.level.as_deref().unwrap_or("-");
        println!("[{level}] {}", l.text);
    })
    .await?;
    println!("Игра завершилась: {status}");
    Ok(())
}
