use console::style;
use indicatif::{ProgressBar, ProgressStyle};

pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} Б")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

pub fn bytes_bar(total: u64, msg: &str) -> ProgressBar {
    let bar = ProgressBar::new(total);
    bar.set_style(
        ProgressStyle::with_template("{msg:12} [{bar:30.green/dim}] {binary_bytes}/{binary_total_bytes} {binary_bytes_per_sec} ~{eta}")
            .unwrap()
            .progress_chars("=> "),
    );
    bar.set_message(msg.to_owned());
    bar
}

pub fn count_bar(total: u64, msg: &str) -> ProgressBar {
    let bar = ProgressBar::new(total);
    bar.set_style(
        ProgressStyle::with_template("{msg:12} [{bar:30.green/dim}] {pos}/{len}")
            .unwrap()
            .progress_chars("=> "),
    );
    bar.set_message(msg.to_owned());
    bar
}

pub fn step(text: impl std::fmt::Display) {
    println!("{} {text}", style("›").cyan());
}

pub fn ok(text: impl std::fmt::Display) {
    println!("{} {text}", style("✓").green().bold());
}

pub fn warn(text: impl std::fmt::Display) {
    println!("{} {text}", style("!").yellow().bold());
}

/// Печатает до `limit` строк, остальное — «…и ещё N».
pub fn list<I: IntoIterator<Item = String>>(prefix: &str, items: I, limit: usize) {
    let items: Vec<String> = items.into_iter().collect();
    for item in items.iter().take(limit) {
        println!("    {prefix} {item}");
    }
    if items.len() > limit {
        println!(
            "    {}",
            style(format!("…и ещё {}", items.len() - limit)).dim()
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes() {
        assert_eq!(super::size(512), "512 Б");
        assert_eq!(super::size(1536), "1.5 КБ");
        assert_eq!(super::size(5 * 1024 * 1024), "5.0 МБ");
    }
}
