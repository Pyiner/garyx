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
            label: "Inbox review".to_owned(),
            table_name: "inbox".to_owned(),
            event_type: "record.created".to_owned(),
            title_template: "New {record_id}".to_owned(),
            body_template: "Review {table_name}".to_owned(),
            agent_id: Some("codex".to_owned()),
            workspace_dir: None,
            enabled: true,
        })
        .unwrap();
    let triggers = service
        .list_data_triggers(Some("inbox".to_owned()), None)
        .unwrap();
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].label, "Inbox review");
    service.drop_table("inbox", None).unwrap();
    assert!(
        service
            .list_data_triggers(Some("inbox".to_owned()), None)
            .unwrap()
            .is_empty()
    );
}
