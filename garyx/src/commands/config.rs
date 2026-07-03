use super::*;

pub(crate) fn cmd_config_show(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    match load_config_or_default(config_path, ConfigRuntimeOverrides::default()) {
        Ok(loaded) => {
            print_diagnostics(&loaded.diagnostics);
            println!("{}", serde_json::to_string_pretty(&loaded.config)?);
        }
        Err(err) => {
            print_errors(&err.diagnostics);
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) fn cmd_config_validate(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    match load_config_or_default(config_path, ConfigRuntimeOverrides::default()) {
        Ok(loaded) => {
            print_diagnostics(&loaded.diagnostics);
            let plugin_schemas = discover_installed_plugin_schemas()
                .map(|(schemas, _)| schemas)
                .unwrap_or_default();
            let issues = validate_channel_account_configs(&loaded.config, &plugin_schemas);
            if !issues.is_empty() {
                print_config_validation_issues(&issues);
                std::process::exit(1);
            }
            println!("Config is valid");
        }
        Err(err) => {
            print_errors(&err.diagnostics);
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) fn cmd_config_path(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(config_path);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    println!("{}", absolute.display());
    Ok(())
}

fn load_config_value(config_path: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    if config_path.exists() {
        let raw = fs::read_to_string(config_path)?;
        Ok(serde_json::from_str::<Value>(&raw)?)
    } else {
        Ok(serde_json::to_value(GaryxConfig::default())?)
    }
}

fn save_config_value(config_path: &Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let _validated: GaryxConfig = serde_json::from_value(value.clone())?;
    write_config_value_atomic(config_path, value, &ConfigWriteOptions::default())?;
    Ok(())
}

pub(super) fn save_config_struct(
    config_path: &Path,
    config: &GaryxConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let value = serde_json::to_value(config)?;
    write_config_value_atomic(config_path, &value, &ConfigWriteOptions::default())?;
    Ok(())
}

fn get_dotted_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.trim().is_empty() {
        return Some(root);
    }
    let mut current = root;
    for seg in path.split('.') {
        match current {
            Value::Object(map) => {
                current = map.get(seg)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

fn set_dotted_path(root: &mut Value, path: &str, value: Value) -> Result<(), String> {
    if path.trim().is_empty() {
        *root = value;
        return Ok(());
    }
    let mut current = root;
    let mut parts = path.split('.').peekable();
    while let Some(seg) = parts.next() {
        if parts.peek().is_none() {
            match current {
                Value::Object(map) => {
                    map.insert(seg.to_owned(), value);
                    return Ok(());
                }
                _ => return Err("target parent is not an object".to_owned()),
            }
        }

        match current {
            Value::Object(map) => {
                let entry = map
                    .entry(seg.to_owned())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if !entry.is_object() {
                    *entry = Value::Object(serde_json::Map::new());
                }
                current = entry;
            }
            _ => return Err("path traverses a non-object value".to_owned()),
        }
    }
    Ok(())
}

fn unset_dotted_path(root: &mut Value, path: &str) -> Result<bool, String> {
    if path.trim().is_empty() {
        return Err("path cannot be empty for unset".to_owned());
    }

    let mut parts: Vec<&str> = path.split('.').collect();
    let key = parts.pop().unwrap_or_default();
    let mut current = root;
    for seg in parts {
        match current {
            Value::Object(map) => {
                let Some(next) = map.get_mut(seg) else {
                    return Ok(false);
                };
                current = next;
            }
            _ => return Err("path traverses a non-object value".to_owned()),
        }
    }

    match current {
        Value::Object(map) => Ok(map.remove(key).is_some()),
        _ => Err("target parent is not an object".to_owned()),
    }
}

pub(crate) fn cmd_config_get(
    config_path: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let value = load_config_value(&prepared.active_path)?;
    let Some(found) = get_dotted_path(&value, path) else {
        eprintln!("Path not found: {path}");
        std::process::exit(1);
    };
    if found.is_string() {
        println!("{}", found.as_str().unwrap_or_default());
    } else {
        println!("{}", serde_json::to_string_pretty(found)?);
    }
    Ok(())
}

pub(crate) fn cmd_config_set(
    config_path: &str,
    path: &str,
    value_text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let mut value = load_config_value(&prepared.active_path)?;
    let new_value = serde_json::from_str::<Value>(value_text)
        .unwrap_or_else(|_| Value::String(value_text.to_owned()));
    if let Err(err) = set_dotted_path(&mut value, path, new_value) {
        eprintln!("Set failed for path '{path}': {err}");
        std::process::exit(1);
    }
    save_config_value(&prepared.active_path, &value)?;
    println!("Updated {path}");
    Ok(())
}

pub(crate) fn cmd_config_unset(
    config_path: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let mut value = load_config_value(&prepared.active_path)?;
    match unset_dotted_path(&mut value, path) {
        Ok(true) => {
            save_config_value(&prepared.active_path, &value)?;
            println!("Removed {path}");
        }
        Ok(false) => {
            eprintln!("Path not found: {path}");
            std::process::exit(1);
        }
        Err(err) => {
            eprintln!("Unset failed for path '{path}': {err}");
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) fn cmd_config_init(
    config_path: &str,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let path = prepared.active_path;
    if path.exists() && !force {
        eprintln!(
            "Config already exists at {}. Use --force to overwrite.",
            path.display()
        );
        std::process::exit(1);
    }
    let default_value = serde_json::to_value(GaryxConfig::default())?;
    write_config_value_atomic(&path, &default_value, &ConfigWriteOptions::default())?;
    println!("Initialized config at {}", path.display());
    Ok(())
}
