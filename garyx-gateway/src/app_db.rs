use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, StatusCode},
};
use base64::{Engine as _, engine::general_purpose};
use chrono::{SecondsFormat, Utc};
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::automation;
use crate::server::AppState;

const SYSTEM_PREFIX: &str = "gx_db_";
const DEFAULT_SQL_LIMIT: usize = 500;
const SYSTEM_FIELDS: [&str; 5] = ["id", "created_at", "updated_at", "created_by", "updated_by"];

#[derive(Debug, thiserror::Error)]
pub enum AppDbError {
    #[error("BadRequest: {0}")]
    BadRequest(String),
    #[error("NotFound: {0}")]
    NotFound(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("database lock poisoned")]
    LockPoisoned,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

type AppDbResult<T> = Result<T, AppDbError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDbFieldSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub not_null: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub indexed: bool,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default, alias = "default")]
    pub default_value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDbFieldView {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub not_null: bool,
    pub unique: bool,
    pub indexed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDbTableSummary {
    pub table_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub schema_version: i64,
    pub record_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDbSchemaView {
    pub table_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub schema_version: i64,
    pub system_fields: Vec<AppDbFieldView>,
    pub fields: Vec<AppDbFieldView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDbEvent {
    pub id: String,
    pub event_type: String,
    pub table_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationDataTrigger {
    pub id: String,
    pub table_name: String,
    pub event_type: String,
    pub title_template: String,
    pub body_template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDbSqlResult {
    pub columns: Vec<String>,
    pub rows: Vec<Value>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateTableBody {
    pub table_name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub fields: Vec<AppDbFieldSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateFieldBody {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub not_null: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub indexed: bool,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default, alias = "default")]
    pub default_value: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecordBody {
    #[serde(default)]
    pub record: Map<String, Value>,
    #[serde(default)]
    pub actor_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SqlQueryBody {
    pub sql: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDataTriggerBody {
    pub table_name: String,
    pub event_type: String,
    pub title_template: String,
    pub body_template: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchDataTriggerBody {
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListEventsQuery {
    #[serde(default)]
    pub table: Option<String>,
    #[serde(default, alias = "eventType")]
    pub event_type: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListDataTriggersQuery {
    #[serde(default)]
    pub table: Option<String>,
    #[serde(default, alias = "eventType")]
    pub event_type: Option<String>,
}

pub struct AppDbService {
    conn: Mutex<Connection>,
}

impl AppDbService {
    pub fn open(path: impl AsRef<Path>) -> AppDbResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        initialize_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    #[cfg(test)]
    pub fn memory() -> AppDbResult<Self> {
        let conn = Connection::open_in_memory()?;
        initialize_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> AppDbResult<MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| AppDbError::LockPoisoned)
    }

    pub fn list_tables(&self) -> AppDbResult<Vec<AppDbTableSummary>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT table_name, display_name, schema_version FROM gx_db_tables ORDER BY table_name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut tables = Vec::new();
        for row in rows {
            let (table_name, display_name, schema_version) = row?;
            let sql = format!("SELECT COUNT(*) FROM {}", quote_ident(&table_name));
            let record_count = conn.query_row(&sql, [], |row| row.get::<_, i64>(0))?;
            tables.push(AppDbTableSummary {
                table_name,
                display_name,
                schema_version,
                record_count,
            });
        }
        Ok(tables)
    }

    pub fn schema(&self, table_name: &str) -> AppDbResult<AppDbSchemaView> {
        let table_name = validate_identifier(table_name, "table_name")?;
        let conn = self.conn()?;
        schema_inner(&conn, &table_name)
    }

    pub fn create_table(
        &self,
        body: CreateTableBody,
        actor_id: Option<String>,
    ) -> AppDbResult<AppDbEvent> {
        let table_name = validate_identifier(&body.table_name, "table_name")?;
        if self.table_exists(&table_name)? {
            return Err(AppDbError::Conflict(format!(
                "table already exists: {table_name}"
            )));
        }
        let fields = normalize_field_specs(body.fields)?;
        let now = now_string();
        {
            let mut conn = self.conn()?;
            let tx = conn.transaction()?;
            let mut column_sql = vec![
                "\"id\" TEXT PRIMARY KEY".to_owned(),
                "\"created_at\" TEXT NOT NULL".to_owned(),
                "\"updated_at\" TEXT NOT NULL".to_owned(),
                "\"created_by\" TEXT".to_owned(),
                "\"updated_by\" TEXT".to_owned(),
            ];
            for field in &fields {
                column_sql.push(field_definition_sql(field, true)?);
            }
            let create_sql = format!(
                "CREATE TABLE {} ({}) STRICT",
                quote_ident(&table_name),
                column_sql.join(", ")
            );
            tx.execute(&create_sql, [])?;
            tx.execute(
                "INSERT INTO gx_db_tables (table_name, display_name, schema_version, created_at, updated_at)
                 VALUES (?1, ?2, 1, ?3, ?3)",
                params![table_name, body.display_name, now],
            )?;
            for field in &fields {
                insert_field_metadata(&tx, &table_name, field, &now)?;
                create_field_indexes(&tx, &table_name, field, false)?;
            }
            tx.commit()?;
        }
        self.log_schema_event(&table_name, actor_id)
    }

    pub fn drop_table(
        &self,
        table_name: &str,
        actor_id: Option<String>,
    ) -> AppDbResult<AppDbEvent> {
        let table_name = validate_identifier(table_name, "table_name")?;
        let before = self.schema(&table_name)?;
        {
            let mut conn = self.conn()?;
            let tx = conn.transaction()?;
            tx.execute(&format!("DROP TABLE {}", quote_ident(&table_name)), [])?;
            tx.execute(
                "DELETE FROM gx_db_fields WHERE table_name = ?1",
                params![table_name],
            )?;
            tx.execute(
                "DELETE FROM gx_db_tables WHERE table_name = ?1",
                params![table_name],
            )?;
            tx.execute(
                "DELETE FROM gx_automation_data_triggers WHERE table_name = ?1",
                params![table_name],
            )?;
            tx.commit()?;
        }
        self.insert_event(AppDbEventInput {
            event_type: "schema.changed".to_owned(),
            table_name,
            record_id: None,
            actor_id,
            schema_version: Some(before.schema_version + 1),
            before: Some(serde_json::to_value(before)?),
            after: None,
        })
    }

    pub fn add_field(
        &self,
        table_name: &str,
        body: CreateFieldBody,
        actor_id: Option<String>,
    ) -> AppDbResult<AppDbEvent> {
        let table_name = validate_identifier(table_name, "table_name")?;
        self.ensure_table(&table_name)?;
        let field = normalize_field_specs(vec![AppDbFieldSpec {
            name: body.name,
            field_type: body.field_type,
            not_null: body.not_null,
            unique: body.unique,
            indexed: body.indexed,
            display_name: body.display_name,
            default_value: body.default_value,
        }])?
        .remove(0);
        if SYSTEM_FIELDS.contains(&field.name.as_str()) {
            return Err(AppDbError::BadRequest(format!(
                "field name is reserved: {}",
                field.name
            )));
        }
        if self.field_exists(&table_name, &field.name)? {
            return Err(AppDbError::Conflict(format!(
                "field already exists: {}.{}",
                table_name, field.name
            )));
        }
        let now = now_string();
        {
            let mut conn = self.conn()?;
            let tx = conn.transaction()?;
            tx.execute(
                &format!(
                    "ALTER TABLE {} ADD COLUMN {}",
                    quote_ident(&table_name),
                    field_definition_sql(&field, false)?
                ),
                [],
            )?;
            insert_field_metadata(&tx, &table_name, &field, &now)?;
            create_field_indexes(&tx, &table_name, &field, true)?;
            tx.execute(
                "UPDATE gx_db_tables SET schema_version = schema_version + 1, updated_at = ?2 WHERE table_name = ?1",
                params![table_name, now],
            )?;
            tx.commit()?;
        }
        self.log_schema_event(&table_name, actor_id)
    }

    pub fn drop_field(
        &self,
        table_name: &str,
        field_name: &str,
        actor_id: Option<String>,
    ) -> AppDbResult<AppDbEvent> {
        let table_name = validate_identifier(table_name, "table_name")?;
        let field_name = validate_identifier(field_name, "field_name")?;
        if SYSTEM_FIELDS.contains(&field_name.as_str()) {
            return Err(AppDbError::BadRequest(format!(
                "system field cannot be dropped: {field_name}"
            )));
        }
        self.ensure_table(&table_name)?;
        if !self.field_exists(&table_name, &field_name)? {
            return Err(AppDbError::NotFound(format!(
                "field not found: {table_name}.{field_name}"
            )));
        }
        let now = now_string();
        {
            let mut conn = self.conn()?;
            let tx = conn.transaction()?;
            drop_known_indexes(&tx, &table_name, &field_name)?;
            tx.execute(
                &format!(
                    "ALTER TABLE {} DROP COLUMN {}",
                    quote_ident(&table_name),
                    quote_ident(&field_name)
                ),
                [],
            )?;
            tx.execute(
                "DELETE FROM gx_db_fields WHERE table_name = ?1 AND field_name = ?2",
                params![table_name, field_name],
            )?;
            tx.execute(
                "UPDATE gx_db_tables SET schema_version = schema_version + 1, updated_at = ?2 WHERE table_name = ?1",
                params![table_name, now],
            )?;
            tx.commit()?;
        }
        self.log_schema_event(&table_name, actor_id)
    }

    pub fn insert_record(
        &self,
        table_name: &str,
        body: RecordBody,
        header_actor: Option<String>,
    ) -> AppDbResult<(Value, AppDbEvent)> {
        let table_name = validate_identifier(table_name, "table_name")?;
        self.ensure_table(&table_name)?;
        let actor_id = body.actor_id.or(header_actor);
        let fields = self.field_types(&table_name)?;
        let mut record = body.record;
        let record_id = record
            .remove("id")
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(new_record_id);
        validate_record_id(&record_id)?;
        let now = now_string();
        let mut columns = vec![
            "id".to_owned(),
            "created_at".to_owned(),
            "updated_at".to_owned(),
            "created_by".to_owned(),
            "updated_by".to_owned(),
        ];
        let mut values = vec![
            SqlValue::Text(record_id.clone()),
            SqlValue::Text(now.clone()),
            SqlValue::Text(now.clone()),
            sql_optional_text(actor_id.as_deref()),
            sql_optional_text(actor_id.as_deref()),
        ];
        for (field, value) in record {
            validate_mutable_record_field(&fields, &field)?;
            columns.push(field.clone());
            values.push(json_to_sql_value(
                &value,
                fields.get(&field).map(String::as_str),
            )?);
        }
        let placeholders = (1..=columns.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let column_sql = columns
            .iter()
            .map(|column| quote_ident(column))
            .collect::<Vec<_>>()
            .join(", ");
        {
            let conn = self.conn()?;
            let sql = format!(
                "INSERT INTO {} ({column_sql}) VALUES ({placeholders})",
                quote_ident(&table_name)
            );
            conn.execute(&sql, params_from_iter(values))?;
        }
        let after = self.record_by_id(&table_name, &record_id)?;
        let event = self.insert_event(AppDbEventInput {
            event_type: "record.created".to_owned(),
            table_name: table_name.clone(),
            record_id: Some(record_id),
            actor_id,
            schema_version: Some(self.schema_version(&table_name)?),
            before: None,
            after: Some(after.clone()),
        })?;
        Ok((after, event))
    }

    pub fn get_record(&self, table_name: &str, record_id: &str) -> AppDbResult<Value> {
        let table_name = validate_identifier(table_name, "table_name")?;
        validate_record_id(record_id)?;
        self.ensure_table(&table_name)?;
        self.record_by_id(&table_name, record_id)
    }

    pub fn update_record(
        &self,
        table_name: &str,
        record_id: &str,
        body: RecordBody,
        header_actor: Option<String>,
    ) -> AppDbResult<(Value, AppDbEvent)> {
        let table_name = validate_identifier(table_name, "table_name")?;
        validate_record_id(record_id)?;
        self.ensure_table(&table_name)?;
        let actor_id = body.actor_id.or(header_actor);
        let before = self.record_by_id(&table_name, record_id)?;
        let fields = self.field_types(&table_name)?;
        if body.record.is_empty() {
            return Err(AppDbError::BadRequest(
                "record update requires at least one field".to_owned(),
            ));
        }
        let now = now_string();
        let mut assignments = Vec::new();
        let mut values = Vec::new();
        for (field, value) in body.record {
            validate_mutable_record_field(&fields, &field)?;
            assignments.push(format!("{} = ?", quote_ident(&field)));
            values.push(json_to_sql_value(
                &value,
                fields.get(&field).map(String::as_str),
            )?);
        }
        assignments.push("\"updated_at\" = ?".to_owned());
        values.push(SqlValue::Text(now));
        assignments.push("\"updated_by\" = ?".to_owned());
        values.push(sql_optional_text(actor_id.as_deref()));
        values.push(SqlValue::Text(record_id.to_owned()));
        {
            let conn = self.conn()?;
            let sql = format!(
                "UPDATE {} SET {} WHERE \"id\" = ?",
                quote_ident(&table_name),
                assignments.join(", ")
            );
            let changed = conn.execute(&sql, params_from_iter(values))?;
            if changed == 0 {
                return Err(AppDbError::NotFound(format!(
                    "record not found: {record_id}"
                )));
            }
        }
        let after = self.record_by_id(&table_name, record_id)?;
        let event = self.insert_event(AppDbEventInput {
            event_type: "record.updated".to_owned(),
            table_name: table_name.clone(),
            record_id: Some(record_id.to_owned()),
            actor_id,
            schema_version: Some(self.schema_version(&table_name)?),
            before: Some(before),
            after: Some(after.clone()),
        })?;
        Ok((after, event))
    }

    pub fn delete_record(
        &self,
        table_name: &str,
        record_id: &str,
        actor_id: Option<String>,
    ) -> AppDbResult<AppDbEvent> {
        let table_name = validate_identifier(table_name, "table_name")?;
        validate_record_id(record_id)?;
        self.ensure_table(&table_name)?;
        let before = self.record_by_id(&table_name, record_id)?;
        {
            let conn = self.conn()?;
            let changed = conn.execute(
                &format!("DELETE FROM {} WHERE \"id\" = ?1", quote_ident(&table_name)),
                params![record_id],
            )?;
            if changed == 0 {
                return Err(AppDbError::NotFound(format!(
                    "record not found: {record_id}"
                )));
            }
        }
        self.insert_event(AppDbEventInput {
            event_type: "record.deleted".to_owned(),
            table_name: table_name.clone(),
            record_id: Some(record_id.to_owned()),
            actor_id,
            schema_version: Some(self.schema_version(&table_name)?),
            before: Some(before),
            after: None,
        })
    }

    pub fn sql_query(&self, sql: &str, limit: Option<usize>) -> AppDbResult<AppDbSqlResult> {
        let sql = sql.trim();
        if sql.is_empty() {
            return Err(AppDbError::BadRequest("sql is empty".to_owned()));
        }
        let limit = limit.unwrap_or(DEFAULT_SQL_LIMIT).min(2_000);
        let conn = self.conn()?;
        let mut stmt = conn.prepare(sql)?;
        if !stmt.readonly() {
            return Err(AppDbError::BadRequest(
                "write SQL is rejected; use garyx db table/field/record commands".to_owned(),
            ));
        }
        let column_count = stmt.column_count();
        let columns = (0..column_count)
            .map(|index| {
                stmt.column_name(index)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|_| format!("column_{index}"))
            })
            .collect::<Vec<_>>();
        let mut rows = Vec::new();
        let mut query = stmt.query([])?;
        while let Some(row) = query.next()? {
            if rows.len() >= limit {
                return Ok(AppDbSqlResult {
                    columns,
                    rows,
                    truncated: true,
                });
            }
            let mut object = Map::new();
            for (index, column) in columns.iter().enumerate() {
                object.insert(column.clone(), sql_ref_to_json(row.get_ref(index)?));
            }
            rows.push(Value::Object(object));
        }
        Ok(AppDbSqlResult {
            columns,
            rows,
            truncated: false,
        })
    }

    pub fn list_events(
        &self,
        table_name: Option<String>,
        event_type: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> AppDbResult<Vec<AppDbEvent>> {
        let table_name = table_name
            .map(|value| validate_identifier(&value, "table"))
            .transpose()?;
        let event_type = event_type
            .map(|value| validate_event_type(&value).map(|_| value))
            .transpose()?;
        let limit = limit.unwrap_or(50).min(500) as i64;
        let offset = offset.unwrap_or(0) as i64;
        let conn = self.conn()?;
        let mut sql = "SELECT id, event_type, table_name, record_id, actor_type, actor_id, thread_id, task_id, schema_version, before_json, after_json, created_at FROM gx_db_events".to_owned();
        let mut clauses = Vec::new();
        let mut bind_values = Vec::new();
        if let Some(table_name) = table_name {
            clauses.push("table_name = ?".to_owned());
            bind_values.push(SqlValue::Text(table_name));
        }
        if let Some(event_type) = event_type {
            clauses.push("event_type = ?".to_owned());
            bind_values.push(SqlValue::Text(event_type));
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
        bind_values.push(SqlValue::Integer(limit));
        bind_values.push(SqlValue::Integer(offset));
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(bind_values), event_from_row)?;
        collect_rows(rows)
    }

    pub fn create_data_trigger(
        &self,
        body: CreateDataTriggerBody,
    ) -> AppDbResult<AutomationDataTrigger> {
        let table_name = validate_identifier(&body.table_name, "table_name")?;
        self.ensure_table(&table_name)?;
        validate_event_type(&body.event_type)?;
        let agent_id = normalize_optional_string(body.agent_id);
        if let Some(agent_id) = &agent_id {
            validate_agent_id(agent_id)?;
        }
        let workspace_dir = normalize_optional_string(body.workspace_dir);
        let title_template = trim_required(&body.title_template, "title_template")?;
        let body_template = trim_required(&body.body_template, "body_template")?;
        let id = format!("autodata_{}", Uuid::new_v4().simple());
        let now = now_string();
        {
            let conn = self.conn()?;
            conn.execute(
                "INSERT INTO gx_automation_data_triggers
                 (id, table_name, event_type, title_template, body_template, agent_id, workspace_dir, enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                params![
                    id,
                    table_name,
                    body.event_type,
                    title_template,
                    body_template,
                    agent_id,
                    workspace_dir,
                    if body.enabled { 1 } else { 0 },
                    now,
                ],
            )?;
        }
        self.data_trigger_by_id(&id)
    }

    pub fn list_data_triggers(
        &self,
        table_name: Option<String>,
        event_type: Option<String>,
    ) -> AppDbResult<Vec<AutomationDataTrigger>> {
        let table_name = table_name
            .map(|value| validate_identifier(&value, "table"))
            .transpose()?;
        let event_type = event_type
            .map(|value| validate_event_type(&value).map(|_| value))
            .transpose()?;
        let conn = self.conn()?;
        let mut sql = "SELECT id, table_name, event_type, title_template, body_template, agent_id, workspace_dir, enabled, created_at, updated_at FROM gx_automation_data_triggers".to_owned();
        let mut clauses = Vec::new();
        let mut bind_values = Vec::new();
        if let Some(table_name) = table_name {
            clauses.push("table_name = ?".to_owned());
            bind_values.push(SqlValue::Text(table_name));
        }
        if let Some(event_type) = event_type {
            clauses.push("event_type = ?".to_owned());
            bind_values.push(SqlValue::Text(event_type));
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(bind_values), data_trigger_from_row)?;
        collect_rows(rows)
    }

    pub fn patch_data_trigger(
        &self,
        trigger_id: &str,
        body: PatchDataTriggerBody,
    ) -> AppDbResult<AutomationDataTrigger> {
        validate_data_trigger_id(trigger_id)?;
        let Some(enabled) = body.enabled else {
            return Err(AppDbError::BadRequest(
                "trigger patch requires enabled".to_owned(),
            ));
        };
        let now = now_string();
        {
            let conn = self.conn()?;
            let changed = conn.execute(
                "UPDATE gx_automation_data_triggers SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
                params![trigger_id, if enabled { 1 } else { 0 }, now],
            )?;
            if changed == 0 {
                return Err(AppDbError::NotFound(format!(
                    "trigger not found: {trigger_id}"
                )));
            }
        }
        self.data_trigger_by_id(trigger_id)
    }

    pub fn delete_data_trigger(&self, trigger_id: &str) -> AppDbResult<()> {
        validate_data_trigger_id(trigger_id)?;
        let conn = self.conn()?;
        let changed = conn.execute(
            "DELETE FROM gx_automation_data_triggers WHERE id = ?1",
            params![trigger_id],
        )?;
        if changed == 0 {
            return Err(AppDbError::NotFound(format!(
                "trigger not found: {trigger_id}"
            )));
        }
        Ok(())
    }

    pub fn attach_task_to_event(
        &self,
        event_id: &str,
        task_id: &str,
        thread_id: &str,
    ) -> AppDbResult<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE gx_db_events SET task_id = ?2, thread_id = ?3 WHERE id = ?1",
            params![event_id, task_id, thread_id],
        )?;
        Ok(())
    }

    fn table_exists(&self, table_name: &str) -> AppDbResult<bool> {
        let conn = self.conn()?;
        let exists = conn
            .query_row(
                "SELECT 1 FROM gx_db_tables WHERE table_name = ?1",
                params![table_name],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(exists)
    }

    fn ensure_table(&self, table_name: &str) -> AppDbResult<()> {
        if self.table_exists(table_name)? {
            Ok(())
        } else {
            Err(AppDbError::NotFound(format!(
                "table not found: {table_name}"
            )))
        }
    }

    fn field_exists(&self, table_name: &str, field_name: &str) -> AppDbResult<bool> {
        let conn = self.conn()?;
        let exists = conn
            .query_row(
                "SELECT 1 FROM gx_db_fields WHERE table_name = ?1 AND field_name = ?2",
                params![table_name, field_name],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(exists)
    }

    fn field_types(&self, table_name: &str) -> AppDbResult<HashMap<String, String>> {
        let conn = self.conn()?;
        field_types_inner(&conn, table_name)
    }

    fn schema_version(&self, table_name: &str) -> AppDbResult<i64> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT schema_version FROM gx_db_tables WHERE table_name = ?1",
            params![table_name],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| AppDbError::NotFound(format!("table not found: {table_name}")))
    }

    fn record_by_id(&self, table_name: &str, record_id: &str) -> AppDbResult<Value> {
        let conn = self.conn()?;
        let sql = format!(
            "SELECT * FROM {} WHERE \"id\" = ?1",
            quote_ident(table_name)
        );
        let mut stmt = conn.prepare(&sql)?;
        let columns = statement_columns(&stmt);
        let row = stmt
            .query_row(params![record_id], |row| row_to_json(row, &columns))
            .optional()?;
        row.ok_or_else(|| AppDbError::NotFound(format!("record not found: {record_id}")))
    }

    fn data_trigger_by_id(&self, trigger_id: &str) -> AppDbResult<AutomationDataTrigger> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id, table_name, event_type, title_template, body_template, agent_id, workspace_dir, enabled, created_at, updated_at
             FROM gx_automation_data_triggers WHERE id = ?1",
            params![trigger_id],
            data_trigger_from_row,
        )
        .optional()?
        .ok_or_else(|| AppDbError::NotFound(format!("trigger not found: {trigger_id}")))
    }

    fn log_schema_event(
        &self,
        table_name: &str,
        actor_id: Option<String>,
    ) -> AppDbResult<AppDbEvent> {
        let schema = self.schema(table_name)?;
        let event = self.insert_event(AppDbEventInput {
            event_type: "schema.changed".to_owned(),
            table_name: table_name.to_owned(),
            record_id: None,
            actor_id,
            schema_version: Some(schema.schema_version),
            before: None,
            after: Some(serde_json::to_value(&schema)?),
        })?;
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO gx_db_schema_versions (id, table_name, version, schema_json, created_at, event_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                format!("dbver_{}", Uuid::new_v4().simple()),
                table_name,
                schema.schema_version,
                serde_json::to_string(&schema)?,
                event.created_at,
                event.id,
            ],
        )?;
        Ok(event)
    }

    fn insert_event(&self, input: AppDbEventInput) -> AppDbResult<AppDbEvent> {
        validate_event_type(&input.event_type)?;
        let id = format!("dbevt_{}", Uuid::new_v4().simple());
        let now = now_string();
        let actor_id = normalize_optional_string(input.actor_id);
        let actor_type = actor_id.as_ref().map(|_| "agent_or_cli".to_owned());
        let before_json = input
            .before
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let after_json = input
            .after
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        {
            let conn = self.conn()?;
            conn.execute(
                "INSERT INTO gx_db_events
                 (id, event_type, table_name, record_id, actor_type, actor_id, schema_version, before_json, after_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    input.event_type,
                    input.table_name,
                    input.record_id,
                    actor_type,
                    actor_id,
                    input.schema_version,
                    before_json,
                    after_json,
                    now,
                ],
            )?;
        }
        self.event_by_id(&id)
    }

    fn event_by_id(&self, event_id: &str) -> AppDbResult<AppDbEvent> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id, event_type, table_name, record_id, actor_type, actor_id, thread_id, task_id, schema_version, before_json, after_json, created_at
             FROM gx_db_events WHERE id = ?1",
            params![event_id],
            event_from_row,
        )
        .optional()?
        .ok_or_else(|| AppDbError::NotFound(format!("event not found: {event_id}")))
    }
}

#[derive(Debug)]
struct AppDbEventInput {
    event_type: String,
    table_name: String,
    record_id: Option<String>,
    actor_id: Option<String>,
    schema_version: Option<i64>,
    before: Option<Value>,
    after: Option<Value>,
}

fn initialize_connection(conn: &Connection) -> AppDbResult<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS gx_db_tables (
            table_name TEXT PRIMARY KEY,
            display_name TEXT,
            schema_version INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS gx_db_fields (
            table_name TEXT NOT NULL,
            field_name TEXT NOT NULL,
            type TEXT NOT NULL,
            not_null INTEGER NOT NULL DEFAULT 0,
            default_json TEXT,
            unique_field INTEGER NOT NULL DEFAULT 0,
            indexed INTEGER NOT NULL DEFAULT 0,
            display_name TEXT,
            created_at TEXT NOT NULL,
            PRIMARY KEY (table_name, field_name)
        ) STRICT;

        CREATE TABLE IF NOT EXISTS gx_db_schema_versions (
            id TEXT PRIMARY KEY,
            table_name TEXT NOT NULL,
            version INTEGER NOT NULL,
            schema_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            event_id TEXT NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS gx_db_events (
            id TEXT PRIMARY KEY,
            event_type TEXT NOT NULL,
            table_name TEXT NOT NULL,
            record_id TEXT,
            actor_type TEXT,
            actor_id TEXT,
            thread_id TEXT,
            task_id TEXT,
            schema_version INTEGER,
            before_json TEXT,
            after_json TEXT,
            created_at TEXT NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS gx_automation_data_triggers (
            id TEXT PRIMARY KEY,
            table_name TEXT NOT NULL,
            event_type TEXT NOT NULL,
            title_template TEXT NOT NULL,
            body_template TEXT NOT NULL,
            agent_id TEXT,
            workspace_dir TEXT,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        ) STRICT;
        "#,
    )?;
    Ok(())
}

fn schema_inner(conn: &Connection, table_name: &str) -> AppDbResult<AppDbSchemaView> {
    let table = conn
        .query_row(
            "SELECT display_name, schema_version FROM gx_db_tables WHERE table_name = ?1",
            params![table_name],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?
        .ok_or_else(|| AppDbError::NotFound(format!("table not found: {table_name}")))?;
    let mut stmt = conn.prepare(
        "SELECT field_name, type, not_null, unique_field, indexed, display_name, default_json
         FROM gx_db_fields WHERE table_name = ?1 ORDER BY rowid",
    )?;
    let fields = stmt.query_map(params![table_name], field_from_row)?;
    Ok(AppDbSchemaView {
        table_name: table_name.to_owned(),
        display_name: table.0,
        schema_version: table.1,
        system_fields: system_field_views(),
        fields: collect_rows(fields)?,
    })
}

fn field_types_inner(conn: &Connection, table_name: &str) -> AppDbResult<HashMap<String, String>> {
    let mut stmt =
        conn.prepare("SELECT field_name, type FROM gx_db_fields WHERE table_name = ?1")?;
    let rows = stmt.query_map(params![table_name], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut fields = HashMap::new();
    for row in rows {
        let (name, field_type) = row?;
        fields.insert(name, field_type);
    }
    Ok(fields)
}

fn field_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AppDbFieldView> {
    let default_json = row.get::<_, Option<String>>(6)?;
    let default_value = default_json.and_then(|value| serde_json::from_str::<Value>(&value).ok());
    Ok(AppDbFieldView {
        name: row.get(0)?,
        field_type: row.get(1)?,
        not_null: row.get::<_, i64>(2)? != 0,
        unique: row.get::<_, i64>(3)? != 0,
        indexed: row.get::<_, i64>(4)? != 0,
        display_name: row.get(5)?,
        default_value,
    })
}

fn event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AppDbEvent> {
    let before_json = row.get::<_, Option<String>>(9)?;
    let after_json = row.get::<_, Option<String>>(10)?;
    Ok(AppDbEvent {
        id: row.get(0)?,
        event_type: row.get(1)?,
        table_name: row.get(2)?,
        record_id: row.get(3)?,
        actor_type: row.get(4)?,
        actor_id: row.get(5)?,
        thread_id: row.get(6)?,
        task_id: row.get(7)?,
        schema_version: row.get(8)?,
        before: before_json.and_then(|value| serde_json::from_str(&value).ok()),
        after: after_json.and_then(|value| serde_json::from_str(&value).ok()),
        created_at: row.get(11)?,
    })
}

fn data_trigger_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutomationDataTrigger> {
    Ok(AutomationDataTrigger {
        id: row.get(0)?,
        table_name: row.get(1)?,
        event_type: row.get(2)?,
        title_template: row.get(3)?,
        body_template: row.get(4)?,
        agent_id: row.get(5)?,
        workspace_dir: row.get(6)?,
        enabled: row.get::<_, i64>(7)? != 0,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> AppDbResult<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn system_field_views() -> Vec<AppDbFieldView> {
    vec![
        system_field("id", "TEXT", true),
        system_field("created_at", "TEXT", true),
        system_field("updated_at", "TEXT", true),
        system_field("created_by", "TEXT", false),
        system_field("updated_by", "TEXT", false),
    ]
}

fn system_field(name: &str, field_type: &str, not_null: bool) -> AppDbFieldView {
    AppDbFieldView {
        name: name.to_owned(),
        field_type: field_type.to_owned(),
        not_null,
        unique: false,
        indexed: false,
        display_name: None,
        default_value: None,
    }
}

fn normalize_field_specs(fields: Vec<AppDbFieldSpec>) -> AppDbResult<Vec<AppDbFieldSpec>> {
    let mut seen = BTreeMap::new();
    let mut out = Vec::new();
    for field in fields {
        let name = validate_identifier(&field.name, "field_name")?;
        if SYSTEM_FIELDS.contains(&name.as_str()) {
            return Err(AppDbError::BadRequest(format!(
                "field name is reserved: {name}"
            )));
        }
        if seen.insert(name.clone(), ()).is_some() {
            return Err(AppDbError::BadRequest(format!(
                "duplicate field name: {name}"
            )));
        }
        out.push(AppDbFieldSpec {
            name,
            field_type: normalize_sqlite_type(&field.field_type)?,
            not_null: field.not_null,
            unique: field.unique,
            indexed: field.indexed,
            display_name: normalize_optional_string(field.display_name),
            default_value: field.default_value,
        });
    }
    Ok(out)
}

fn field_definition_sql(field: &AppDbFieldSpec, include_unique: bool) -> AppDbResult<String> {
    let mut sql = format!("{} {}", quote_ident(&field.name), field.field_type);
    if field.not_null {
        sql.push_str(" NOT NULL");
    }
    if include_unique && field.unique {
        sql.push_str(" UNIQUE");
    }
    if let Some(default_value) = &field.default_value {
        sql.push_str(" DEFAULT ");
        sql.push_str(&default_sql_literal(default_value)?);
    }
    Ok(sql)
}

fn insert_field_metadata(
    tx: &rusqlite::Transaction<'_>,
    table_name: &str,
    field: &AppDbFieldSpec,
    now: &str,
) -> AppDbResult<()> {
    let default_json = field
        .default_value
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    tx.execute(
        "INSERT INTO gx_db_fields
         (table_name, field_name, type, not_null, default_json, unique_field, indexed, display_name, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            table_name,
            field.name,
            field.field_type,
            if field.not_null { 1 } else { 0 },
            default_json,
            if field.unique { 1 } else { 0 },
            if field.indexed { 1 } else { 0 },
            field.display_name,
            now,
        ],
    )?;
    Ok(())
}

fn create_field_indexes(
    tx: &rusqlite::Transaction<'_>,
    table_name: &str,
    field: &AppDbFieldSpec,
    create_unique_index: bool,
) -> AppDbResult<()> {
    if field.unique && create_unique_index {
        let sql = format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS {} ON {} ({})",
            quote_ident(&format!("gxidx_{table_name}_{}_uniq", field.name)),
            quote_ident(table_name),
            quote_ident(&field.name)
        );
        tx.execute(&sql, [])?;
    } else if field.indexed {
        let sql = format!(
            "CREATE INDEX IF NOT EXISTS {} ON {} ({})",
            quote_ident(&format!("gxidx_{table_name}_{}", field.name)),
            quote_ident(table_name),
            quote_ident(&field.name)
        );
        tx.execute(&sql, [])?;
    }
    Ok(())
}

fn drop_known_indexes(
    tx: &rusqlite::Transaction<'_>,
    table_name: &str,
    field_name: &str,
) -> AppDbResult<()> {
    for suffix in ["", "_uniq"] {
        let sql = format!(
            "DROP INDEX IF EXISTS {}",
            quote_ident(&format!("gxidx_{table_name}_{field_name}{suffix}"))
        );
        tx.execute(&sql, [])?;
    }
    Ok(())
}

fn statement_columns(stmt: &rusqlite::Statement<'_>) -> Vec<String> {
    (0..stmt.column_count())
        .map(|index| {
            stmt.column_name(index)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|_| format!("column_{index}"))
        })
        .collect()
}

fn row_to_json(row: &rusqlite::Row<'_>, columns: &[String]) -> rusqlite::Result<Value> {
    let mut object = Map::new();
    for (index, column) in columns.iter().enumerate() {
        object.insert(column.clone(), sql_ref_to_json(row.get_ref(index)?));
    }
    Ok(Value::Object(object))
}

fn sql_ref_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => json!(value),
        ValueRef::Real(value) => json!(value),
        ValueRef::Text(value) => json!(String::from_utf8_lossy(value).to_string()),
        ValueRef::Blob(value) => json!(general_purpose::STANDARD.encode(value)),
    }
}

fn json_to_sql_value(value: &Value, field_type: Option<&str>) -> AppDbResult<SqlValue> {
    if value.is_null() {
        return Ok(SqlValue::Null);
    }
    match field_type.unwrap_or("ANY") {
        "TEXT" => Ok(SqlValue::Text(match value {
            Value::String(value) => value.clone(),
            _ => value.to_string(),
        })),
        "INTEGER" => {
            if let Some(value) = value.as_i64() {
                Ok(SqlValue::Integer(value))
            } else if let Some(value) = value.as_bool() {
                Ok(SqlValue::Integer(i64::from(value)))
            } else {
                Err(AppDbError::BadRequest(format!(
                    "INTEGER field requires integer value, got {value}"
                )))
            }
        }
        "REAL" => value.as_f64().map(SqlValue::Real).ok_or_else(|| {
            AppDbError::BadRequest(format!("REAL field requires number, got {value}"))
        }),
        "BLOB" => match value {
            Value::String(value) => Ok(SqlValue::Blob(value.as_bytes().to_vec())),
            _ => Err(AppDbError::BadRequest(
                "BLOB field requires string input in the first CLI version".to_owned(),
            )),
        },
        "ANY" => match value {
            Value::Bool(value) => Ok(SqlValue::Integer(i64::from(*value))),
            Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    Ok(SqlValue::Integer(value))
                } else if let Some(value) = value.as_f64() {
                    Ok(SqlValue::Real(value))
                } else {
                    Ok(SqlValue::Text(value.to_string()))
                }
            }
            Value::String(value) => Ok(SqlValue::Text(value.clone())),
            _ => Ok(SqlValue::Text(value.to_string())),
        },
        other => Err(AppDbError::BadRequest(format!(
            "unsupported SQLite type: {other}"
        ))),
    }
}

fn sql_optional_text(value: Option<&str>) -> SqlValue {
    value
        .map(|value| SqlValue::Text(value.to_owned()))
        .unwrap_or(SqlValue::Null)
}

fn validate_mutable_record_field(fields: &HashMap<String, String>, field: &str) -> AppDbResult<()> {
    let field = validate_identifier(field, "record field")?;
    if SYSTEM_FIELDS.contains(&field.as_str()) {
        return Err(AppDbError::BadRequest(format!(
            "system field cannot be written: {field}"
        )));
    }
    if fields.contains_key(&field) {
        Ok(())
    } else {
        Err(AppDbError::BadRequest(format!("unknown field: {field}")))
    }
}

fn validate_identifier(value: &str, label: &str) -> AppDbResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppDbError::BadRequest(format!("{label} is empty")));
    }
    if value.len() > 64 {
        return Err(AppDbError::BadRequest(format!(
            "{label} exceeds 64 characters"
        )));
    }
    if value.starts_with(SYSTEM_PREFIX) {
        return Err(AppDbError::BadRequest(format!(
            "{label} cannot start with reserved prefix {SYSTEM_PREFIX}"
        )));
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(AppDbError::BadRequest(format!("{label} is empty")));
    };
    if !first.is_ascii_lowercase() {
        return Err(AppDbError::BadRequest(format!(
            "{label} must start with a lowercase ASCII letter"
        )));
    }
    if !chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_') {
        return Err(AppDbError::BadRequest(format!(
            "{label} must be snake_case ASCII"
        )));
    }
    if is_reserved_sql_word(value) {
        return Err(AppDbError::BadRequest(format!(
            "{label} is a reserved SQL word: {value}"
        )));
    }
    Ok(value.to_owned())
}

fn validate_record_id(value: &str) -> AppDbResult<()> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 {
        return Err(AppDbError::BadRequest(
            "record id must be 1..128 characters".to_owned(),
        ));
    }
    Ok(())
}

fn validate_data_trigger_id(value: &str) -> AppDbResult<()> {
    let value = value.trim();
    if value.starts_with("autodata_") && value.len() <= 80 {
        Ok(())
    } else {
        Err(AppDbError::BadRequest(format!(
            "invalid trigger id: {value}"
        )))
    }
}

fn validate_agent_id(value: &str) -> AppDbResult<()> {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == ':' || ch == '.')
    {
        Ok(())
    } else {
        Err(AppDbError::BadRequest(format!(
            "agent_id contains unsupported characters: {value}"
        )))
    }
}

fn validate_event_type(value: &str) -> AppDbResult<()> {
    match value {
        "record.created" | "record.updated" | "record.deleted" | "schema.changed" => Ok(()),
        _ => Err(AppDbError::BadRequest(format!(
            "unsupported event_type: {value}"
        ))),
    }
}

fn normalize_sqlite_type(value: &str) -> AppDbResult<String> {
    let value = value.trim().to_ascii_uppercase();
    match value.as_str() {
        "TEXT" | "INTEGER" | "REAL" | "BLOB" | "ANY" => Ok(value),
        _ => Err(AppDbError::BadRequest(format!(
            "type must be one of TEXT, INTEGER, REAL, BLOB, ANY; got {value}"
        ))),
    }
}

fn default_sql_literal(value: &Value) -> AppDbResult<String> {
    Ok(match value {
        Value::Null => "NULL".to_owned(),
        Value::Bool(value) => {
            if *value {
                "1".to_owned()
            } else {
                "0".to_owned()
            }
        }
        Value::Number(value) => value.to_string(),
        Value::String(value) => sql_string_literal(value),
        Value::Array(_) | Value::Object(_) => sql_string_literal(&value.to_string()),
    })
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn now_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn new_record_id() -> String {
    format!("rec_{}", Uuid::new_v4().simple())
}

fn default_true() -> bool {
    true
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}

fn trim_required(value: &str, label: &str) -> AppDbResult<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(AppDbError::BadRequest(format!("{label} is empty")))
    } else {
        Ok(value.to_owned())
    }
}

fn is_reserved_sql_word(value: &str) -> bool {
    matches!(
        value,
        "abort"
            | "action"
            | "add"
            | "alter"
            | "and"
            | "as"
            | "by"
            | "case"
            | "check"
            | "column"
            | "create"
            | "delete"
            | "drop"
            | "from"
            | "group"
            | "index"
            | "insert"
            | "into"
            | "join"
            | "limit"
            | "not"
            | "null"
            | "or"
            | "order"
            | "pragma"
            | "primary"
            | "select"
            | "set"
            | "table"
            | "update"
            | "values"
            | "where"
    )
}

fn actor_from_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-garyx-actor")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn db_service(state: &Arc<AppState>) -> Arc<AppDbService> {
    state.ops.app_db.clone()
}

pub async fn list_tables(State(state): State<Arc<AppState>>) -> (StatusCode, Json<Value>) {
    db_result(
        db_service(&state).list_tables(),
        |tables| json!({ "tables": tables }),
    )
}

pub async fn create_table(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateTableBody>,
) -> (StatusCode, Json<Value>) {
    let service = db_service(&state);
    match service.create_table(body, actor_from_header(&headers)) {
        Ok(event) => {
            let trigger_results = automation::run_data_triggers_for_db_event(state, &event).await;
            (
                StatusCode::CREATED,
                Json(json!({ "event": event, "triggeredTasks": trigger_results })),
            )
        }
        Err(error) => db_error_response(error),
    }
}

pub async fn get_schema(
    State(state): State<Arc<AppState>>,
    AxumPath(table): AxumPath<String>,
) -> (StatusCode, Json<Value>) {
    db_result(db_service(&state).schema(&table), |schema| json!(schema))
}

pub async fn drop_table(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(table): AxumPath<String>,
) -> (StatusCode, Json<Value>) {
    let service = db_service(&state);
    match service.drop_table(&table, actor_from_header(&headers)) {
        Ok(event) => {
            let trigger_results = automation::run_data_triggers_for_db_event(state, &event).await;
            (
                StatusCode::OK,
                Json(json!({ "event": event, "triggeredTasks": trigger_results })),
            )
        }
        Err(error) => db_error_response(error),
    }
}

pub async fn add_field(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(table): AxumPath<String>,
    Json(body): Json<CreateFieldBody>,
) -> (StatusCode, Json<Value>) {
    let service = db_service(&state);
    match service.add_field(&table, body, actor_from_header(&headers)) {
        Ok(event) => {
            let trigger_results = automation::run_data_triggers_for_db_event(state, &event).await;
            (
                StatusCode::CREATED,
                Json(json!({ "event": event, "triggeredTasks": trigger_results })),
            )
        }
        Err(error) => db_error_response(error),
    }
}

pub async fn drop_field(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((table, field)): AxumPath<(String, String)>,
) -> (StatusCode, Json<Value>) {
    let service = db_service(&state);
    match service.drop_field(&table, &field, actor_from_header(&headers)) {
        Ok(event) => {
            let trigger_results = automation::run_data_triggers_for_db_event(state, &event).await;
            (
                StatusCode::OK,
                Json(json!({ "event": event, "triggeredTasks": trigger_results })),
            )
        }
        Err(error) => db_error_response(error),
    }
}

pub async fn insert_record(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(table): AxumPath<String>,
    Json(body): Json<RecordBody>,
) -> (StatusCode, Json<Value>) {
    let service = db_service(&state);
    match service.insert_record(&table, body, actor_from_header(&headers)) {
        Ok((record, event)) => {
            let trigger_results = automation::run_data_triggers_for_db_event(state, &event).await;
            (
                StatusCode::CREATED,
                Json(
                    json!({ "record": record, "event": event, "triggeredTasks": trigger_results }),
                ),
            )
        }
        Err(error) => db_error_response(error),
    }
}

pub async fn get_record(
    State(state): State<Arc<AppState>>,
    AxumPath((table, id)): AxumPath<(String, String)>,
) -> (StatusCode, Json<Value>) {
    db_result(
        db_service(&state).get_record(&table, &id),
        |record| json!({ "record": record }),
    )
}

pub async fn update_record(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((table, id)): AxumPath<(String, String)>,
    Json(body): Json<RecordBody>,
) -> (StatusCode, Json<Value>) {
    let service = db_service(&state);
    match service.update_record(&table, &id, body, actor_from_header(&headers)) {
        Ok((record, event)) => {
            let trigger_results = automation::run_data_triggers_for_db_event(state, &event).await;
            (
                StatusCode::OK,
                Json(
                    json!({ "record": record, "event": event, "triggeredTasks": trigger_results }),
                ),
            )
        }
        Err(error) => db_error_response(error),
    }
}

pub async fn delete_record(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath((table, id)): AxumPath<(String, String)>,
) -> (StatusCode, Json<Value>) {
    let service = db_service(&state);
    match service.delete_record(&table, &id, actor_from_header(&headers)) {
        Ok(event) => {
            let trigger_results = automation::run_data_triggers_for_db_event(state, &event).await;
            (
                StatusCode::OK,
                Json(json!({ "event": event, "triggeredTasks": trigger_results })),
            )
        }
        Err(error) => db_error_response(error),
    }
}

pub async fn sql_query(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SqlQueryBody>,
) -> (StatusCode, Json<Value>) {
    db_result(
        db_service(&state).sql_query(&body.sql, body.limit),
        |result| json!(result),
    )
}

pub async fn list_events(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListEventsQuery>,
) -> (StatusCode, Json<Value>) {
    db_result(
        db_service(&state).list_events(query.table, query.event_type, query.limit, query.offset),
        |events| json!({ "events": events }),
    )
}

fn db_result<T>(result: AppDbResult<T>, ok: impl FnOnce(T) -> Value) -> (StatusCode, Json<Value>) {
    db_result_status(StatusCode::OK, result, ok)
}

fn db_result_status<T>(
    status: StatusCode,
    result: AppDbResult<T>,
    ok: impl FnOnce(T) -> Value,
) -> (StatusCode, Json<Value>) {
    match result {
        Ok(value) => (status, Json(ok(value))),
        Err(error) => db_error_response(error),
    }
}

fn db_error_response(error: AppDbError) -> (StatusCode, Json<Value>) {
    let (status, code) = match &error {
        AppDbError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BadRequest"),
        AppDbError::NotFound(_) => (StatusCode::NOT_FOUND, "NotFound"),
        AppDbError::Conflict(_) | AppDbError::Sqlite(rusqlite::Error::SqliteFailure(_, _)) => {
            (StatusCode::CONFLICT, "Conflict")
        }
        AppDbError::LockPoisoned
        | AppDbError::Io(_)
        | AppDbError::Sqlite(_)
        | AppDbError::Serde(_) => (StatusCode::INTERNAL_SERVER_ERROR, "InternalError"),
    };
    (
        status,
        Json(json!({
            "error": code,
            "message": error.to_string(),
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_strict_table_and_rejects_write_sql() {
        let service = AppDbService::memory().unwrap();
        service
            .create_table(
                CreateTableBody {
                    table_name: "contacts".to_owned(),
                    display_name: Some("Contacts".to_owned()),
                    fields: vec![
                        AppDbFieldSpec {
                            name: "name".to_owned(),
                            field_type: "TEXT".to_owned(),
                            not_null: true,
                            unique: false,
                            indexed: true,
                            display_name: None,
                            default_value: None,
                        },
                        AppDbFieldSpec {
                            name: "score".to_owned(),
                            field_type: "REAL".to_owned(),
                            not_null: false,
                            unique: false,
                            indexed: false,
                            display_name: None,
                            default_value: None,
                        },
                    ],
                },
                Some("test-agent".to_owned()),
            )
            .unwrap();
        let (record, event) = service
            .insert_record(
                "contacts",
                RecordBody {
                    record: Map::from_iter([
                        ("name".to_owned(), json!("Test User")),
                        ("score".to_owned(), json!(9.5)),
                    ]),
                    actor_id: None,
                },
                Some("test-agent".to_owned()),
            )
            .unwrap();
        assert_eq!(event.event_type, "record.created");
        assert_eq!(record["name"], "Test User");
        let query = service
            .sql_query("select name, score from contacts", None)
            .unwrap();
        assert_eq!(query.rows.len(), 1);
        let error = service.sql_query("insert into contacts (id) values ('x')", None);
        assert!(matches!(error, Err(AppDbError::BadRequest(_))));
    }

    #[test]
    fn logs_before_and_after_for_record_update() {
        let service = AppDbService::memory().unwrap();
        service
            .create_table(
                CreateTableBody {
                    table_name: "notes".to_owned(),
                    display_name: None,
                    fields: vec![AppDbFieldSpec {
                        name: "body".to_owned(),
                        field_type: "TEXT".to_owned(),
                        not_null: false,
                        unique: false,
                        indexed: false,
                        display_name: None,
                        default_value: None,
                    }],
                },
                None,
            )
            .unwrap();
        let (record, _) = service
            .insert_record(
                "notes",
                RecordBody {
                    record: Map::from_iter([("body".to_owned(), json!("first"))]),
                    actor_id: None,
                },
                None,
            )
            .unwrap();
        let record_id = record["id"].as_str().unwrap().to_owned();
        let (_, event) = service
            .update_record(
                "notes",
                &record_id,
                RecordBody {
                    record: Map::from_iter([("body".to_owned(), json!("second"))]),
                    actor_id: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(event.before.as_ref().unwrap()["body"], "first");
        assert_eq!(event.after.as_ref().unwrap()["body"], "second");
    }

    #[test]
    fn can_add_unique_field_after_table_creation() {
        let service = AppDbService::memory().unwrap();
        service
            .create_table(
                CreateTableBody {
                    table_name: "accounts".to_owned(),
                    display_name: None,
                    fields: vec![],
                },
                None,
            )
            .unwrap();
        service
            .add_field(
                "accounts",
                CreateFieldBody {
                    name: "email".to_owned(),
                    field_type: "TEXT".to_owned(),
                    not_null: false,
                    unique: true,
                    indexed: false,
                    display_name: None,
                    default_value: None,
                },
                None,
            )
            .unwrap();
        let schema = service.schema("accounts").unwrap();
        assert!(
            schema
                .fields
                .iter()
                .any(|field| field.name == "email" && field.unique)
        );
    }

    #[test]
    fn enforces_safe_table_and_field_names() {
        let service = AppDbService::memory().unwrap();
        let error = service.create_table(
            CreateTableBody {
                table_name: "BadName".to_owned(),
                display_name: None,
                fields: vec![],
            },
            None,
        );
        assert!(matches!(error, Err(AppDbError::BadRequest(_))));
    }

    #[test]
    fn dropping_table_removes_its_triggers() {
        let service = AppDbService::memory().unwrap();
        service
            .create_table(
                CreateTableBody {
                    table_name: "inbox".to_owned(),
                    display_name: None,
                    fields: vec![AppDbFieldSpec {
                        name: "title".to_owned(),
                        field_type: "TEXT".to_owned(),
                        not_null: false,
                        unique: false,
                        indexed: false,
                        display_name: None,
                        default_value: None,
                    }],
                },
                None,
            )
            .unwrap();
        service
            .create_data_trigger(CreateDataTriggerBody {
                table_name: "inbox".to_owned(),
                event_type: "record.created".to_owned(),
                title_template: "New {record_id}".to_owned(),
                body_template: "Review {table_name}".to_owned(),
                agent_id: Some("codex".to_owned()),
                workspace_dir: None,
                enabled: true,
            })
            .unwrap();
        assert_eq!(
            service
                .list_data_triggers(Some("inbox".to_owned()), None)
                .unwrap()
                .len(),
            1
        );
        service.drop_table("inbox", None).unwrap();
        assert!(
            service
                .list_data_triggers(Some("inbox".to_owned()), None)
                .unwrap()
                .is_empty()
        );
    }
}
