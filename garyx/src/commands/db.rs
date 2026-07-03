use super::*;

// ---------------------------------------------------------------------------
// App database commands
// ---------------------------------------------------------------------------
fn parse_db_field_spec(spec: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let (name, field_type) = spec
        .split_once(':')
        .ok_or_else(|| format!("field spec must be name:TYPE, got {spec}"))?;
    Ok(json!({
        "name": trim_required_cli(name, "field name")?,
        "type": trim_required_cli(field_type, "field type")?,
    }))
}

fn parse_json_object(
    input: &str,
    label: &str,
) -> Result<Map<String, Value>, Box<dyn std::error::Error>> {
    let value = serde_json::from_str::<Value>(input)?;
    match value {
        Value::Object(object) => Ok(object),
        _ => Err(format!("{label} must be a JSON object").into()),
    }
}

fn parse_optional_json_value(
    input: Option<String>,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    input
        .map(|value| serde_json::from_str::<Value>(&value).map_err(Into::into))
        .transpose()
}

fn db_print_table_list(payload: &Value) {
    let tables = payload["tables"].as_array().cloned().unwrap_or_default();
    if tables.is_empty() {
        println!("Tables: (none)");
        return;
    }
    println!(
        "{:<32}  {:<8}  {:<8}  DISPLAY",
        "TABLE", "VERSION", "RECORDS"
    );
    println!("{}", "-".repeat(72));
    for table in tables {
        println!(
            "{:<32}  {:<8}  {:<8}  {}",
            table["table_name"].as_str().unwrap_or("-"),
            table["schema_version"].as_i64().unwrap_or_default(),
            table["record_count"].as_i64().unwrap_or_default(),
            table["display_name"].as_str().unwrap_or("-")
        );
    }
}

fn db_print_schema(payload: &Value) {
    println!("Table: {}", payload["table_name"].as_str().unwrap_or("-"));
    if let Some(display_name) = payload["display_name"].as_str() {
        println!("Display: {display_name}");
    }
    println!(
        "Schema version: {}",
        payload["schema_version"].as_i64().unwrap_or_default()
    );
    println!();
    println!(
        "{:<24}  {:<8}  {:<8}  {:<6}  {:<6}  DISPLAY",
        "FIELD", "TYPE", "NOTNULL", "UNIQ", "INDEX"
    );
    println!("{}", "-".repeat(86));
    for field in payload["system_fields"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(payload["fields"].as_array().into_iter().flatten())
    {
        println!(
            "{:<24}  {:<8}  {:<8}  {:<6}  {:<6}  {}",
            field["name"].as_str().unwrap_or("-"),
            field["type"].as_str().unwrap_or("-"),
            field["not_null"].as_bool().unwrap_or(false),
            field["unique"].as_bool().unwrap_or(false),
            field["indexed"].as_bool().unwrap_or(false),
            field["display_name"].as_str().unwrap_or("-"),
        );
    }
}

fn db_print_sql_result(payload: &Value) {
    let rows = payload["rows"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        println!("Rows: (none)");
    } else {
        for row in rows {
            println!(
                "{}",
                serde_json::to_string(&row).unwrap_or_else(|_| row.to_string())
            );
        }
    }
    if payload["truncated"].as_bool().unwrap_or(false) {
        println!("Result truncated");
    }
}

fn db_print_events(payload: &Value) {
    let events = payload["events"].as_array().cloned().unwrap_or_default();
    if events.is_empty() {
        println!("Events: (none)");
        return;
    }
    println!(
        "{:<34}  {:<15}  {:<24}  {:<24}  RECORD",
        "EVENT", "TYPE", "TABLE", "CREATED"
    );
    println!("{}", "-".repeat(118));
    for event in events {
        println!(
            "{:<34}  {:<15}  {:<24}  {:<24}  {}",
            event["id"].as_str().unwrap_or("-"),
            event["event_type"].as_str().unwrap_or("-"),
            event["table_name"].as_str().unwrap_or("-"),
            event["created_at"].as_str().unwrap_or("-"),
            event["record_id"].as_str().unwrap_or("-"),
        );
    }
}

pub(crate) async fn cmd_db_table_list(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, "/api/db/tables").await?;
    if json {
        return print_pretty_json(&payload);
    }
    db_print_table_list(&payload);
    Ok(())
}

pub(crate) async fn cmd_db_table_create(
    config_path: &str,
    table: &str,
    display_name: Option<String>,
    fields: Vec<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let fields = fields
        .iter()
        .map(|field| parse_db_field_spec(field))
        .collect::<Result<Vec<_>, _>>()?;
    let mut body = json!({
        "table_name": table,
        "fields": fields,
    });
    if let Some(display_name) = trim_optional_cli(display_name) {
        body["display_name"] = json!(display_name);
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json_as_cli_actor(&gateway, "/api/db/tables", &body).await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!(
        "Created table: {}",
        payload["event"]["table_name"].as_str().unwrap_or("-")
    );
    Ok(())
}

pub(crate) async fn cmd_db_table_schema(
    config_path: &str,
    table: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/db/tables/{}", urlencoding::encode(&table)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    db_print_schema(&payload);
    Ok(())
}

pub(crate) async fn cmd_db_table_drop(
    config_path: &str,
    table: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/db/tables/{}", urlencoding::encode(&table)),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Dropped table: {table}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn cmd_db_field_add(
    config_path: &str,
    table: &str,
    field: &str,
    field_type: &str,
    not_null: bool,
    unique: bool,
    indexed: bool,
    display_name: Option<String>,
    default_value: Option<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let mut body = json!({
        "name": trim_required_cli(field, "field")?,
        "type": trim_required_cli(field_type, "type")?,
        "not_null": not_null,
        "unique": unique,
        "indexed": indexed,
    });
    if let Some(display_name) = trim_optional_cli(display_name) {
        body["display_name"] = json!(display_name);
    }
    if let Some(default_value) = parse_optional_json_value(default_value)? {
        body["default"] = default_value;
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/db/tables/{}/fields", urlencoding::encode(&table)),
        &body,
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!(
        "Added field: {table}.{}",
        body["name"].as_str().unwrap_or("-")
    );
    Ok(())
}

pub(crate) async fn cmd_db_field_drop(
    config_path: &str,
    table: &str,
    field: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let field = trim_required_cli(field, "field")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json_as_cli_actor(
        &gateway,
        &format!(
            "/api/db/tables/{}/fields/{}",
            urlencoding::encode(&table),
            urlencoding::encode(&field)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Dropped field: {table}.{field}");
    Ok(())
}

pub(crate) async fn cmd_db_record_insert(
    config_path: &str,
    table: &str,
    data: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let body = json!({ "record": parse_json_object(data, "data")? });
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json_as_cli_actor(
        &gateway,
        &format!("/api/db/tables/{}/records", urlencoding::encode(&table)),
        &body,
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!(
        "Inserted record: {}",
        payload["record"]["id"].as_str().unwrap_or("-")
    );
    Ok(())
}

pub(crate) async fn cmd_db_record_get(
    config_path: &str,
    table: &str,
    id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let id = trim_required_cli(id, "id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!(
            "/api/db/tables/{}/records/{}",
            urlencoding::encode(&table),
            urlencoding::encode(&id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    print_pretty_json(&payload["record"])?;
    Ok(())
}

pub(crate) async fn cmd_db_record_update(
    config_path: &str,
    table: &str,
    id: &str,
    data: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let id = trim_required_cli(id, "id")?;
    let body = json!({ "record": parse_json_object(data, "data")? });
    let gateway = gateway_endpoint(config_path)?;
    let payload = patch_gateway_json_as_cli_actor(
        &gateway,
        &format!(
            "/api/db/tables/{}/records/{}",
            urlencoding::encode(&table),
            urlencoding::encode(&id)
        ),
        &body,
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Updated record: {id}");
    Ok(())
}

pub(crate) async fn cmd_db_record_delete(
    config_path: &str,
    table: &str,
    id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let table = trim_required_cli(table, "table")?;
    let id = trim_required_cli(id, "id")?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json_as_cli_actor(
        &gateway,
        &format!(
            "/api/db/tables/{}/records/{}",
            urlencoding::encode(&table),
            urlencoding::encode(&id)
        ),
    )
    .await?;
    if json {
        return print_pretty_json(&payload);
    }
    println!("Deleted record: {id}");
    Ok(())
}

pub(crate) async fn cmd_db_sql(
    config_path: &str,
    sql: Vec<String>,
    limit: Option<usize>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let sql = trim_required_cli(&sql.join(" "), "sql")?;
    let mut body = json!({ "sql": sql });
    if let Some(limit) = limit {
        body["limit"] = json!(limit);
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = post_gateway_json(&gateway, "/api/db/sql", &body).await?;
    if json {
        return print_pretty_json(&payload);
    }
    db_print_sql_result(&payload);
    Ok(())
}

pub(crate) async fn cmd_db_events(
    config_path: &str,
    table: Option<String>,
    event_type: Option<String>,
    limit: usize,
    offset: usize,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut query = vec![format!("limit={limit}"), format!("offset={offset}")];
    if let Some(table) = trim_optional_cli(table) {
        query.push(format!("table={}", urlencoding::encode(&table)));
    }
    if let Some(event_type) = trim_optional_cli(event_type) {
        query.push(format!("eventType={}", urlencoding::encode(&event_type)));
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload =
        fetch_gateway_json(&gateway, &format!("/api/db/events?{}", query.join("&"))).await?;
    if json {
        return print_pretty_json(&payload);
    }
    db_print_events(&payload);
    Ok(())
}
