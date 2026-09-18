//! Разбор вывода игры. Конфиг логирования Mojang пишет события log4j в XML.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogLine {
    /// `INFO`, `WARN`, `ERROR`…; `None` — обычная строка вывода.
    pub level: Option<String>,
    pub thread: Option<String>,
    pub text: String,
    /// Пришло из stderr.
    pub stderr: bool,
}

impl LogLine {
    pub fn raw(text: String, stderr: bool) -> Self {
        let level = stderr.then(|| "ERROR".to_owned());
        Self {
            level,
            thread: None,
            text,
            stderr,
        }
    }
}

/// Собирает многострочные `<log4j:Event>…</log4j:Event>` в [`LogLine`].
#[derive(Default)]
pub struct LogParser {
    event: Option<String>,
}

impl LogParser {
    pub fn push(&mut self, line: &str) -> Option<LogLine> {
        if let Some(buf) = &mut self.event {
            buf.push('\n');
            buf.push_str(line);
            if line.contains("</log4j:Event>") {
                let event = self.event.take().unwrap();
                return Some(parse_event(&event));
            }
            return None;
        }
        if line.trim_start().starts_with("<log4j:Event") {
            if line.contains("</log4j:Event>") {
                return Some(parse_event(line));
            }
            self.event = Some(line.to_owned());
            return None;
        }
        Some(LogLine::raw(line.to_owned(), false))
    }
}

fn attr(event: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let start = event.find(&key)? + key.len();
    let end = event[start..].find('"')? + start;
    Some(unescape(&event[start..end]))
}

fn cdata(event: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}><![CDATA[");
    let start = event.find(&open)? + open.len();
    let end = event[start..].find("]]>")? + start;
    Some(event[start..end].to_owned())
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn parse_event(event: &str) -> LogLine {
    let mut text = cdata(event, "log4j:Message").unwrap_or_default();
    if let Some(t) = cdata(event, "log4j:Throwable") {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(t.trim_end());
    }
    LogLine {
        level: attr(event, "level"),
        thread: attr(event, "thread"),
        text,
        stderr: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_events() {
        let mut p = LogParser::default();
        assert_eq!(
            p.push(
                r#"<log4j:Event logger="ekb" timestamp="1" level="INFO" thread="Render thread">"#
            ),
            None
        );
        assert_eq!(
            p.push("  <log4j:Message><![CDATA[Setting user: Steve]]></log4j:Message>"),
            None
        );
        let line = p.push("</log4j:Event>").unwrap();
        assert_eq!(line.level.as_deref(), Some("INFO"));
        assert_eq!(line.thread.as_deref(), Some("Render thread"));
        assert_eq!(line.text, "Setting user: Steve");

        p.push(r#"<log4j:Event logger="x" level="ERROR" thread="main &quot;1&quot;">"#);
        p.push("<log4j:Message><![CDATA[Crash\nsecond line]]></log4j:Message>");
        p.push("<log4j:Throwable><![CDATA[java.lang.RuntimeException: boom");
        p.push("\tat a.b(C.java:1)\n]]></log4j:Throwable>");
        let line = p.push("</log4j:Event>").unwrap();
        assert_eq!(line.level.as_deref(), Some("ERROR"));
        assert_eq!(line.thread.as_deref(), Some("main \"1\""));
        assert!(
            line.text
                .starts_with("Crash\nsecond line\njava.lang.RuntimeException: boom")
        );
    }

    #[test]
    fn plain_lines() {
        let mut p = LogParser::default();
        let l = p.push("[12:00:00] [main/INFO]: plain").unwrap();
        assert_eq!(l.level, None);
        assert_eq!(l.text, "[12:00:00] [main/INFO]: plain");
    }
}
