//! Статус сервера Minecraft — тот же запрос, что делает игра в списке серверов (1.7+).

use anyhow::{Context, Result, bail};
use scam_core::server::{DEFAULT_PORT, ServerAddress};
use serde::Serialize;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub online: u32,
    pub max: u32,
    pub version: String,
    pub motd: String,
    pub latency_ms: u64,
}

/// Куда реально подключаться: SRV-запись `_minecraft._tcp.<host>`, если порт не указан.
pub async fn resolve(addr: &ServerAddress) -> (String, u16) {
    if let Some(port) = addr.port {
        return (addr.host.clone(), port);
    }
    if !addr.is_ip()
        && let Ok(Some(found)) = timeout(Duration::from_secs(2), srv(&addr.host)).await
    {
        return found;
    }
    (addr.host.clone(), DEFAULT_PORT)
}

async fn srv(host: &str) -> Option<(String, u16)> {
    let resolver = hickory_resolver::Resolver::builder_tokio()
        .ok()?
        .build()
        .ok()?;
    let lookup = resolver
        .srv_lookup(format!("_minecraft._tcp.{host}."))
        .await
        .ok()?;
    lookup
        .answers()
        .iter()
        .filter_map(|r| match &r.data {
            hickory_resolver::proto::rr::RData::SRV(s) => Some(s),
            _ => None,
        })
        .min_by_key(|s| s.priority)
        .map(|s| (s.target.to_utf8().trim_end_matches('.').to_owned(), s.port))
}

pub async fn ping(addr: &ServerAddress) -> Result<Status> {
    timeout(Duration::from_secs(6), ping_inner(addr))
        .await
        .context("сервер не ответил вовремя")?
}

async fn ping_inner(addr: &ServerAddress) -> Result<Status> {
    let (host, port) = resolve(addr).await;
    let mut stream = timeout(
        Duration::from_secs(4),
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    .context("сервер не отвечает")?
    .context("не удалось подключиться к серверу")?;

    // Handshake: id 0, протокол -1 («любой»), адрес, порт, следующее состояние 1 (статус).
    let mut hs = Vec::new();
    write_varint(&mut hs, 0);
    write_varint(&mut hs, -1);
    write_string(&mut hs, &addr.host);
    hs.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut hs, 1);
    let mut out = Vec::new();
    write_varint(&mut out, hs.len() as i32);
    out.extend(hs);
    // Status Request: пакет из одного id 0.
    out.extend([1, 0]);

    let started = Instant::now();
    stream.write_all(&out).await?;
    let len = read_varint(&mut stream).await? as usize;
    let latency_ms = started.elapsed().as_millis() as u64;
    if !(1..=2 * 1024 * 1024).contains(&len) {
        bail!("странный ответ сервера");
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    let mut cur = &body[..];
    if read_varint(&mut cur).await? != 0 {
        bail!("странный ответ сервера");
    }
    let json_len = read_varint(&mut cur).await? as usize;
    let json = cur.get(..json_len).context("обрезанный ответ сервера")?;
    parse_status(json, latency_ms)
}

fn parse_status(json: &[u8], latency_ms: u64) -> Result<Status> {
    let v: serde_json::Value = serde_json::from_slice(json).context("сервер прислал не JSON")?;
    Ok(Status {
        online: v["players"]["online"].as_u64().unwrap_or(0) as u32,
        max: v["players"]["max"].as_u64().unwrap_or(0) as u32,
        version: strip_formatting(v["version"]["name"].as_str().unwrap_or("")),
        motd: strip_formatting(&chat_text(&v["description"]))
            .trim()
            .to_owned(),
        latency_ms,
    })
}

/// Текст из чат-компонента: строка или `{text, extra: [...]}`.
fn chat_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items.iter().map(chat_text).collect(),
        serde_json::Value::Object(o) => {
            let mut s = o
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_owned();
            if let Some(extra) = o.get("extra") {
                s.push_str(&chat_text(extra));
            }
            s
        }
        _ => String::new(),
    }
}

/// Убирает цветовые коды `§x`.
fn strip_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

fn write_varint(buf: &mut Vec<u8>, value: i32) {
    let mut v = value as u32;
    loop {
        if v & !0x7f == 0 {
            buf.push(v as u8);
            return;
        }
        buf.push((v & 0x7f | 0x80) as u8);
        v >>= 7;
    }
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    write_varint(buf, s.len() as i32);
    buf.extend_from_slice(s.as_bytes());
}

async fn read_varint<R: AsyncRead + Unpin>(r: &mut R) -> Result<i32> {
    let mut value = 0u32;
    for i in 0..5 {
        let b = r.read_u8().await.context("сервер оборвал соединение")?;
        value |= ((b & 0x7f) as u32) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(value as i32);
        }
    }
    bail!("слишком длинное число в ответе сервера")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn varints() {
        for v in [0, 1, 127, 128, 25565, -1, i32::MAX] {
            let mut buf = Vec::new();
            write_varint(&mut buf, v);
            assert_eq!(read_varint(&mut &buf[..]).await.unwrap(), v);
        }
        let mut buf = Vec::new();
        write_varint(&mut buf, -1);
        assert_eq!(buf, [0xff, 0xff, 0xff, 0xff, 0x0f]);
    }

    #[test]
    fn status_json() {
        let s = parse_status(
            r#"{"version":{"name":"Paper 1.20.1","protocol":763},"players":{"max":100,"online":12},
                "description":{"text":"§aДобро ","extra":[{"text":"пожаловать"},"!"]}}"#
                .as_bytes(),
            42,
        )
        .unwrap();
        assert_eq!(
            s,
            Status {
                online: 12,
                max: 100,
                version: "Paper 1.20.1".into(),
                motd: "Добро пожаловать!".into(),
                latency_ms: 42
            }
        );
        let s = parse_status(r#"{"description":"§6Old §lserver"}"#.as_bytes(), 1).unwrap();
        assert_eq!(s.motd, "Old server");
    }

    /// Мини-сервер, который отвечает как настоящий Minecraft.
    #[tokio::test]
    async fn ping_local_server() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // handshake + status request
            let len = read_varint(&mut sock).await.unwrap() as usize;
            let mut hs = vec![0u8; len];
            sock.read_exact(&mut hs).await.unwrap();
            let len = read_varint(&mut sock).await.unwrap() as usize;
            let mut req = vec![0u8; len];
            sock.read_exact(&mut req).await.unwrap();
            assert_eq!(req, [0]);
            let json = r#"{"version":{"name":"1.20.1"},"players":{"max":20,"online":3},"description":"Тест"}"#;
            let mut body = Vec::new();
            write_varint(&mut body, 0);
            write_string(&mut body, json);
            let mut out = Vec::new();
            write_varint(&mut out, body.len() as i32);
            out.extend(body);
            sock.write_all(&out).await.unwrap();
        });
        let addr = ServerAddress::parse(&format!("127.0.0.1:{port}")).unwrap();
        let s = ping(&addr).await.unwrap();
        assert_eq!((s.online, s.max, s.motd.as_str()), (3, 20, "Тест"));
    }

    /// Живой сервер: `cargo test -p scam-mc -- --ignored real_server`
    #[tokio::test]
    #[ignore]
    async fn real_server() {
        let addr = ServerAddress::parse("mc.hypixel.net").unwrap();
        println!("SRV/адрес: {:?}", resolve(&addr).await);
        let s = ping(&addr).await.unwrap();
        println!("{s:?}");
        assert!(s.max > 0);
    }

    #[tokio::test]
    async fn closed_port_is_error() {
        let addr = ServerAddress::parse("127.0.0.1:9").unwrap();
        assert!(ping(&addr).await.is_err());
    }
}
