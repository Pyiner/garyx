use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::diagnostics::ConfigDiagnostics;

const MAX_INCLUDE_DEPTH: usize = 10;

/// Process `$include` directives in a JSON value tree.
///
/// When an object contains `"$include": "path/to/file.json"`, the referenced
/// file is loaded, parsed, and its top-level keys are merged into the object.
/// Keys already present in the object take precedence over included keys.
/// The `$include` key is removed after processing.
///
/// Supports nested includes up to [`MAX_INCLUDE_DEPTH`] levels.
pub fn process_includes(value: &mut Value, base_dir: &Path, diagnostics: &mut ConfigDiagnostics) {
    process_includes_inner(value, base_dir, diagnostics, 0, &mut Vec::new());
}

fn process_includes_inner(
    value: &mut Value,
    base_dir: &Path,
    diagnostics: &mut ConfigDiagnostics,
    depth: usize,
    include_stack: &mut Vec<PathBuf>,
) {
    if depth > MAX_INCLUDE_DEPTH {
        diagnostics.push_error(
            "CONFIG_INCLUDE_DEPTH",
            format!("$include depth exceeds maximum of {MAX_INCLUDE_DEPTH}"),
            None::<String>,
        );
        return;
    }

    match value {
        Value::Object(map) => {
            // First, handle $include at this level.
            if let Some(include_val) = map.remove("$include") {
                let include_path_str = match include_val.as_str() {
                    Some(s) => s.to_owned(),
                    None => {
                        diagnostics.push_error(
                            "CONFIG_INCLUDE_TYPE",
                            "$include value must be a string",
                            None::<String>,
                        );
                        return;
                    }
                };

                let include_path = if Path::new(&include_path_str).is_absolute() {
                    PathBuf::from(&include_path_str)
                } else {
                    base_dir.join(&include_path_str)
                };

                // Circular include detection.
                let canonical =
                    fs::canonicalize(&include_path).unwrap_or_else(|_| include_path.clone());
                if include_stack.contains(&canonical) {
                    diagnostics.push_error(
                        "CONFIG_INCLUDE_CIRCULAR",
                        format!("circular $include detected: {}", include_path.display()),
                        None::<String>,
                    );
                    return;
                }

                match fs::read_to_string(&include_path) {
                    Ok(raw) => match serde_json::from_str::<Value>(&raw) {
                        Ok(mut included) => {
                            let inc_base = include_path
                                .parent()
                                .map(Path::to_path_buf)
                                .unwrap_or_else(|| base_dir.to_path_buf());

                            include_stack.push(canonical);
                            process_includes_inner(
                                &mut included,
                                &inc_base,
                                diagnostics,
                                depth + 1,
                                include_stack,
                            );
                            include_stack.pop();

                            // Merge included keys - existing keys take precedence.
                            if let Some(inc_map) = included.as_object() {
                                for (k, v) in inc_map {
                                    map.entry(k.clone()).or_insert_with(|| v.clone());
                                }
                            }
                        }
                        Err(e) => {
                            diagnostics.push_error(
                                "CONFIG_INCLUDE_PARSE",
                                format!(
                                    "failed to parse included file {}: {e}",
                                    include_path.display()
                                ),
                                None::<String>,
                            );
                        }
                    },
                    Err(e) => {
                        diagnostics.push_error(
                            "CONFIG_INCLUDE_IO",
                            format!(
                                "failed to read included file {}: {e}",
                                include_path.display()
                            ),
                            None::<String>,
                        );
                    }
                }
            }

            // Recurse into remaining child values.
            for v in map.values_mut() {
                process_includes_inner(v, base_dir, diagnostics, depth, include_stack);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                process_includes_inner(v, base_dir, diagnostics, depth, include_stack);
            }
        }
        _ => {}
    }
}
