//! Командная строка и запуск игры.

use crate::dirs::GameDirs;
use crate::install::Prepared;
use crate::log::{LogLine, LogParser};
use crate::offline;
use crate::rules;
use crate::version::Arg;
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    /// Папка игры (экземпляр сборки).
    pub game_dir: PathBuf,
    pub player: String,
    pub memory_max_mb: u32,
    pub memory_min_mb: Option<u32>,
    pub extra_jvm_args: Vec<String>,
    pub launcher_name: String,
    pub launcher_version: String,
}

/// Такие же флаги сборщика мусора ставит официальный лаунчер.
const DEFAULT_JVM_ARGS: &[&str] = &[
    "-XX:+UnlockExperimentalVMOptions",
    "-XX:+UseG1GC",
    "-XX:G1NewSizePercent=20",
    "-XX:G1ReservePercent=20",
    "-XX:MaxGCPauseMillis=50",
    "-XX:G1HeapRegionSize=32M",
];

fn substitute(s: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                match vars.get(key) {
                    Some(v) => out.push_str(v),
                    None => out.push_str(&rest[start..start + 2 + end + 1]),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn expand(args: &[Arg], env: &rules::Env, vars: &HashMap<&str, String>) -> Vec<String> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Arg::Plain(s) => out.push(substitute(s, vars)),
            Arg::Ruled { rules: r, value } if rules::allowed(r, env) => {
                out.extend(value.values().into_iter().map(|v| substitute(v, vars)));
            }
            Arg::Ruled { .. } => {}
        }
    }
    out
}

/// Аргументы после пути к java.
pub fn command_line(dirs: &GameDirs, p: &Prepared, o: &LaunchOptions) -> Result<Vec<String>> {
    if !offline::is_valid_nick(&o.player) {
        bail!("ник «{}»: 3–16 символов, латиница, цифры и _", o.player);
    }
    let v = &p.version;
    let sep = if cfg!(windows) { ";" } else { ":" };
    let classpath = p
        .classpath
        .iter()
        .map(|c| c.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(sep);
    let path = |p: &PathBuf| p.to_string_lossy().into_owned();

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("natives_directory", path(&p.natives_dir));
    vars.insert("launcher_name", o.launcher_name.clone());
    vars.insert("launcher_version", o.launcher_version.clone());
    vars.insert("classpath", classpath);
    vars.insert("classpath_separator", sep.to_owned());
    vars.insert("library_directory", path(&dirs.libraries()));
    vars.insert("version_name", v.id.clone());
    vars.insert("auth_player_name", o.player.clone());
    vars.insert("game_directory", path(&o.game_dir));
    vars.insert("assets_root", path(&dirs.assets()));
    vars.insert("game_assets", path(&dirs.assets()));
    vars.insert(
        "assets_index_name",
        v.asset_index
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_default(),
    );
    vars.insert("auth_uuid", offline::offline_uuid(&o.player));
    vars.insert("auth_access_token", "0".into());
    vars.insert("auth_session", "token:0".into());
    vars.insert("clientid", String::new());
    vars.insert("auth_xuid", String::new());
    vars.insert("user_type", "msa".into());
    vars.insert("user_properties", "{}".into());
    vars.insert(
        "version_type",
        v.kind.clone().unwrap_or_else(|| "release".into()),
    );

    let mut args: Vec<String> = Vec::new();
    if let Some(min) = o.memory_min_mb {
        args.push(format!("-Xms{min}M"));
    }
    args.push(format!("-Xmx{}M", o.memory_max_mb));
    args.extend(DEFAULT_JVM_ARGS.iter().map(|s| s.to_string()));
    args.extend(o.extra_jvm_args.iter().cloned());

    match v.arguments.as_ref().filter(|a| !a.jvm.is_empty()) {
        Some(a) => args.extend(expand(&a.jvm, &p.env, &vars)),
        None => {
            args.push(substitute(
                "-Djava.library.path=${natives_directory}",
                &vars,
            ));
            args.push("-cp".into());
            args.push(vars["classpath"].clone());
        }
    }
    if let Some(log) = v.logging.as_ref().and_then(|l| l.client.as_ref()) {
        let file = path(&dirs.log_config(&log.file.id));
        args.push(log.argument.replace("${path}", &file));
    }

    args.push(
        v.main_class
            .clone()
            .context("в JSON версии нет mainClass")?,
    );

    match (&v.arguments, &v.minecraft_arguments) {
        (Some(a), _) if !a.game.is_empty() => args.extend(expand(&a.game, &p.env, &vars)),
        (_, Some(legacy)) => args.extend(legacy.split_whitespace().map(|s| substitute(s, &vars))),
        _ => bail!("в JSON версии нет аргументов игры"),
    }
    Ok(args)
}

/// Запускает игру. stdout/stderr — в пайпы, читать через [`pump_logs`].
pub fn launch(dirs: &GameDirs, p: &Prepared, o: &LaunchOptions) -> Result<Child> {
    let args = command_line(dirs, p, o)?;
    std::fs::create_dir_all(&o.game_dir)?;
    let mut cmd = Command::new(&p.java);
    cmd.args(&args)
        .current_dir(&o.game_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .with_context(|| format!("не удалось запустить {}", p.java.display()))
}

/// Читает stdout и stderr игры, пока она их не закроет.
pub async fn read_logs(
    stdout: ChildStdout,
    stderr: ChildStderr,
    mut on_line: impl FnMut(LogLine),
) -> Result<()> {
    let mut out = BufReader::new(stdout).lines();
    let mut err = BufReader::new(stderr).lines();
    let mut out_parser = LogParser::default();
    let (mut out_done, mut err_done) = (false, false);

    while !(out_done && err_done) {
        tokio::select! {
            line = out.next_line(), if !out_done => match line? {
                Some(l) => {
                    if let Some(parsed) = out_parser.push(&l) {
                        on_line(parsed);
                    }
                }
                None => out_done = true,
            },
            line = err.next_line(), if !err_done => match line? {
                Some(l) => on_line(LogLine::raw(l, true)),
                None => err_done = true,
            },
        }
    }
    Ok(())
}

/// Читает вывод игры до её завершения.
pub async fn pump_logs(child: &mut Child, on_line: impl FnMut(LogLine)) -> Result<ExitStatus> {
    let stdout = child.stdout.take().context("нет stdout")?;
    let stderr = child.stderr.take().context("нет stderr")?;
    read_logs(stdout, stderr, on_line).await?;
    Ok(child.wait().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitution() {
        let mut vars = HashMap::new();
        vars.insert("a", "1".to_string());
        vars.insert("natives_directory", "/n".to_string());
        assert_eq!(substitute("x${a}y${a}", &vars), "x1y1");
        assert_eq!(
            substitute("-Djava.library.path=${natives_directory}/java", &vars),
            "-Djava.library.path=/n/java"
        );
        assert_eq!(substitute("${unknown}", &vars), "${unknown}");
        assert_eq!(substitute("${broken", &vars), "${broken");
        assert_eq!(substitute("plain", &vars), "plain");
    }
}
