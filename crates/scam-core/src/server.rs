//! Адрес сервера Minecraft: `host`, `host:port`, `[v6]:port`.

use std::fmt;

pub const DEFAULT_PORT: u16 = 25565;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    pub host: String,
    /// Порт, если указан явно (иначе — SRV-запись или 25565).
    pub port: Option<u16>,
}

impl ServerAddress {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("пустой адрес сервера".into());
        }
        let (host, port) = if let Some(rest) = s.strip_prefix('[') {
            let (host, tail) = rest.split_once(']').ok_or("нет закрывающей ]")?;
            match tail.strip_prefix(':') {
                Some(p) => (host, Some(p)),
                None if tail.is_empty() => (host, None),
                None => return Err(format!("странный адрес «{s}»")),
            }
        } else if s.matches(':').count() == 1 {
            let (h, p) = s.split_once(':').unwrap();
            (h, Some(p))
        } else {
            // Без двоеточий или IPv6 без скобок — порт не указан.
            (s, None)
        };
        if host.is_empty()
            || !host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
        {
            return Err(format!("странный адрес сервера «{s}»"));
        }
        let port = match port {
            Some(p) => Some(
                p.parse::<u16>()
                    .ok()
                    .filter(|p| *p > 0)
                    .ok_or_else(|| format!("странный порт «{p}»"))?,
            ),
            None => None,
        };
        Ok(Self {
            host: host.to_owned(),
            port,
        })
    }

    pub fn is_ip(&self) -> bool {
        self.host.parse::<std::net::IpAddr>().is_ok()
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        match self.port {
            Some(p) => write!(f, "{host}:{p}"),
            None => f.write_str(&host),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing() {
        let a = ServerAddress::parse(" mc.example.com ").unwrap();
        assert_eq!((a.host.as_str(), a.port), ("mc.example.com", None));
        let a = ServerAddress::parse("mc.example.com:25570").unwrap();
        assert_eq!(a.port, Some(25570));
        assert_eq!(a.to_string(), "mc.example.com:25570");
        let a = ServerAddress::parse("[2001:db8::1]:25565").unwrap();
        assert_eq!(a.host, "2001:db8::1");
        assert_eq!(a.to_string(), "[2001:db8::1]:25565");
        assert!(ServerAddress::parse("2001:db8::1").unwrap().is_ip());
        assert!(ServerAddress::parse("127.0.0.1").unwrap().is_ip());
        assert!(ServerAddress::parse("").is_err());
        assert!(ServerAddress::parse("host:0").is_err());
        assert!(ServerAddress::parse("host:99999").is_err());
        assert!(ServerAddress::parse("bad host").is_err());
    }
}
