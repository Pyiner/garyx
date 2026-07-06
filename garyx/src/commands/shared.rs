use super::*;

pub(super) fn trim_required_cli(
    value: &str,
    field: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} cannot be empty").into());
    }
    Ok(trimmed.to_owned())
}

pub(super) fn trim_optional_cli(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(super) fn committed_message(event: &Value) -> Option<&Value> {
    (event.get("type").and_then(Value::as_str) == Some("committed_message"))
        .then(|| event.get("message"))
        .flatten()
}

pub(super) fn committed_assistant_text(event: &Value) -> Option<&str> {
    let message = committed_message(event)?;
    (message.get("role").and_then(Value::as_str) == Some("assistant"))
        .then(|| {
            message
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| message.get("content").and_then(Value::as_str))
        })
        .flatten()
}

pub(super) fn committed_control_kind(event: &Value) -> Option<&str> {
    committed_message(event)?
        .get("control")
        .and_then(|control| control.get("kind"))
        .and_then(Value::as_str)
}

/// Render a left-aligned text table: a header row, a dash rule, then rows.
///
/// Shared table renderer for human-readable `list` output — new list commands
/// should use this instead of hand-rolling `format!` column widths. Column
/// widths auto-fit the widest cell, columns are separated by two spaces, and
/// the last column is never padded (keeps lines free of trailing spaces). Rows
/// with fewer cells than headers render the missing cells as blanks.
pub(super) fn render_text_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let columns = headers.len();
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in rows {
        for (index, cell) in row.iter().take(columns).enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }
    let render_row = |cells: Vec<&str>| -> String {
        let mut line = String::new();
        for (index, cell) in cells.iter().enumerate().take(columns) {
            if index + 1 == columns {
                line.push_str(cell);
            } else {
                let pad = widths[index].saturating_sub(cell.chars().count());
                line.push_str(cell);
                line.extend(std::iter::repeat_n(' ', pad + 2));
            }
        }
        line.trim_end().to_owned()
    };
    let mut output = String::new();
    output.push_str(&render_row(headers.to_vec()));
    output.push('\n');
    let rule_width = widths.iter().sum::<usize>() + (columns.saturating_sub(1)) * 2;
    output.extend(std::iter::repeat_n('-', rule_width));
    output.push('\n');
    for row in rows {
        let cells = (0..columns)
            .map(|index| row.get(index).map(String::as_str).unwrap_or(""))
            .collect::<Vec<_>>();
        output.push_str(&render_row(cells));
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_text_table_fits_widest_cell_and_pads_columns() {
        let table = render_text_table(
            &["NAME", "STATUS"],
            &[
                vec!["a".to_owned(), "ok".to_owned()],
                vec!["long-name".to_owned(), "warning".to_owned()],
            ],
        );
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines[0], "NAME       STATUS");
        // Rule spans the fully padded width: 9 (NAME) + 2 (gap) + 7 (STATUS).
        assert_eq!(lines[1], "-".repeat(18));
        assert_eq!(lines[2], "a          ok");
        assert_eq!(lines[3], "long-name  warning");
    }

    #[test]
    fn render_text_table_handles_short_rows_without_trailing_spaces() {
        let table = render_text_table(
            &["A", "B", "C"],
            &[
                vec!["x".to_owned()],
                vec!["1".to_owned(), "2".to_owned(), "3".to_owned()],
            ],
        );
        for line in table.lines() {
            assert_eq!(line, line.trim_end(), "no trailing spaces: {line:?}");
        }
        assert!(table.lines().nth(2).is_some_and(|line| line == "x"));
    }
}
