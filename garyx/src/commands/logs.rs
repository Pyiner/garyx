use super::*;

fn default_log_path(path_override: Option<String>) -> String {
    path_override.unwrap_or_else(|| {
        std::env::var("GARYX_LOG_FILE")
            .unwrap_or_else(|_| default_log_file_path().to_string_lossy().to_string())
    })
}

pub(crate) fn cmd_logs_path(path: Option<String>) {
    println!("{}", default_log_path(path));
}

pub(crate) async fn cmd_logs_tail(
    path: Option<String>,
    lines: usize,
    pattern: Option<String>,
    follow: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let log_path = default_log_path(path);
    let mut last_line_count = 0usize;

    loop {
        let content = fs::read_to_string(&log_path).unwrap_or_default();
        let mut all_lines: Vec<&str> = content.lines().collect();
        if let Some(ref p) = pattern {
            all_lines.retain(|line| line.contains(p));
        }

        if !follow {
            let start = all_lines.len().saturating_sub(lines);
            for line in &all_lines[start..] {
                println!("{line}");
            }
            break;
        }

        if all_lines.len() > last_line_count {
            let start = all_lines.len().saturating_sub(lines).max(last_line_count);
            for line in &all_lines[start..] {
                println!("{line}");
            }
            std::io::stdout().flush()?;
            last_line_count = all_lines.len();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Ok(())
}

pub(crate) fn cmd_logs_clear(path: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let log_path = default_log_path(path);
    if let Some(parent) = PathBuf::from(&log_path).parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&log_path, "")?;
    println!("Cleared {}", log_path);
    Ok(())
}
