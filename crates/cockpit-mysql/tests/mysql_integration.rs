use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::Utc;
use cockpit_core::{
    CellValue, CockpitError, ConnectionProfile, DatabaseDriver, DatabaseObjectKind,
    ExecuteQueryRequest, RowMutationKind, RowMutationRequest, TlsOptions,
};
use cockpit_mysql::MySqlDriver;
use uuid::Uuid;

fn test_profile() -> ConnectionProfile {
    let now = Utc::now();
    ConnectionProfile {
        id: Uuid::new_v4(),
        driver_kind: cockpit_core::DatabaseKind::MySql,
        group: None,
        name: "MySQL integration".into(),
        host: std::env::var("COCKPIT_TEST_MYSQL_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        port: std::env::var("COCKPIT_TEST_MYSQL_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(3306),
        username: std::env::var("COCKPIT_TEST_MYSQL_USER").unwrap_or_else(|_| "root".into()),
        database: Some(
            std::env::var("COCKPIT_TEST_MYSQL_DATABASE").unwrap_or_else(|_| "cockpit_test".into()),
        ),
        tls: TlsOptions::default(),
        ssh: None,
        connect_timeout_secs: 10,
        query_timeout_secs: 30,
        pool_size: 2,
        read_only: false,
        production: false,
        color: None,
        created_at: now,
        updated_at: now,
    }
}

fn request(
    sql: impl Into<String>,
    allow_write: bool,
    page_size: usize,
    row_offset: usize,
) -> ExecuteQueryRequest {
    ExecuteQueryRequest {
        execution_id: Uuid::new_v4(),
        sql: sql.into(),
        database: Some("cockpit_test".into()),
        timeout_secs: Some(30),
        allow_write,
        page_size,
        row_offset,
    }
}

#[tokio::test]
#[ignore = "requires COCKPIT_TEST_MYSQL_* or the CI MySQL service"]
async fn metadata_paging_mutation_and_transactions_work() {
    let mut profile = test_profile();
    profile.pool_size = 1;
    let password = std::env::var("COCKPIT_TEST_MYSQL_PASSWORD").unwrap_or_else(|_| "root".into());
    let session = MySqlDriver
        .open(profile, password)
        .await
        .expect("connect to MySQL");
    let version_page = session
        .execute(request("SELECT VERSION()", false, 10, 0))
        .await
        .unwrap();
    let version = match &version_page.rows[0][0] {
        CellValue::Text(value) => value.as_str(),
        value => panic!("unexpected VERSION() value: {value:?}"),
    };
    let supports_invisible_columns = !version.starts_with("5.7.");

    session
        .execute(request("DROP TABLE IF EXISTS cockpit_matrix", true, 100, 0))
        .await
        .unwrap();
    session.execute(request(
        "CREATE TABLE cockpit_matrix (id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY, note VARCHAR(40) NOT NULL)",
        true,
        100,
        0,
    )).await.unwrap();
    for note in ["one", "two", "three"] {
        session
            .mutate_row(RowMutationRequest {
                database: "cockpit_test".into(),
                table: "cockpit_matrix".into(),
                kind: RowMutationKind::Insert,
                values: vec![("note".into(), CellValue::Text(note.into()))],
                key_values: vec![],
                original_values: vec![],
            })
            .await
            .unwrap();
    }

    let tables = session
        .list_tables("cockpit_test", Some("matrix"), 20, 0)
        .await
        .unwrap();
    assert!(tables.iter().any(|table| table.name == "cockpit_matrix"));
    let detail = session
        .table_detail("cockpit_test", "cockpit_matrix")
        .await
        .unwrap();
    assert_eq!(detail.columns.len(), 2);

    session
        .execute(request(
            "CREATE OR REPLACE VIEW cockpit_matrix_view AS SELECT id, note FROM cockpit_matrix",
            true,
            100,
            0,
        ))
        .await
        .unwrap();
    let view = session
        .object_definition(
            "cockpit_test",
            DatabaseObjectKind::View,
            "cockpit_matrix_view",
        )
        .await
        .unwrap();
    assert!(view.ddl.to_ascii_uppercase().contains("CREATE"));

    session
        .execute(request(
            "DROP PROCEDURE IF EXISTS cockpit_matrix_count",
            true,
            100,
            0,
        ))
        .await
        .unwrap();
    session
        .execute(request(
            "CREATE PROCEDURE cockpit_matrix_count() SELECT COUNT(*) FROM cockpit_matrix",
            true,
            100,
            0,
        ))
        .await
        .unwrap();
    let routines = session.list_routines("cockpit_test").await.unwrap();
    assert!(
        routines
            .iter()
            .any(|item| item.name == "cockpit_matrix_count")
    );
    let procedure = session
        .object_definition(
            "cockpit_test",
            DatabaseObjectKind::Procedure,
            "cockpit_matrix_count",
        )
        .await
        .unwrap();
    assert!(procedure.ddl.contains("cockpit_matrix_count"));

    let first = session
        .execute(request(
            "SELECT id, note FROM cockpit_matrix ORDER BY id",
            false,
            2,
            0,
        ))
        .await
        .unwrap();
    assert_eq!(first.rows.len(), 2);
    assert!(first.has_more);
    let second = session
        .execute(request(
            "SELECT id, note FROM cockpit_matrix ORDER BY id",
            false,
            2,
            2,
        ))
        .await
        .unwrap();
    assert_eq!(second.rows.len(), 1);
    assert!(!second.has_more);

    let unchanged = session
        .mutate_row(RowMutationRequest {
            database: "cockpit_test".into(),
            table: "cockpit_matrix".into(),
            kind: RowMutationKind::Update,
            values: vec![("note".into(), CellValue::Text("one".into()))],
            key_values: vec![("id".into(), CellValue::Unsigned("1".into()))],
            original_values: vec![
                ("id".into(), CellValue::Unsigned("1".into())),
                ("note".into(), CellValue::Text("one".into())),
            ],
        })
        .await
        .unwrap();
    assert_eq!(
        unchanged.affected_rows, 1,
        "no-op updates must count matched rows"
    );

    session
        .execute(request("DROP TABLE IF EXISTS cockpit_types", true, 100, 0))
        .await
        .unwrap();
    session
        .execute(request(
            "CREATE TABLE cockpit_types (id INT NOT NULL PRIMARY KEY, source_value INT NOT NULL, computed_value INT GENERATED ALWAYS AS (source_value + 1) STORED, location GEOMETRY NOT NULL)",
            true,
            100,
            0,
        ))
        .await
        .unwrap();
    if supports_invisible_columns {
        session
            .execute(request(
                "ALTER TABLE cockpit_types ADD COLUMN secret VARCHAR(10) INVISIBLE",
                true,
                100,
                0,
            ))
            .await
            .unwrap();
    }
    let types_detail = session
        .table_detail("cockpit_test", "cockpit_types")
        .await
        .unwrap();
    assert!(types_detail.columns.iter().any(|column| {
        column.name == "computed_value"
            && column
                .generation_expression
                .as_deref()
                .is_some_and(|expression| !expression.is_empty())
    }));
    if supports_invisible_columns {
        assert!(types_detail.columns.iter().any(|column| {
            column.name == "secret"
                && column
                    .extra
                    .as_deref()
                    .is_some_and(|extra| extra.to_ascii_uppercase().contains("INVISIBLE"))
        }));
    }
    session
        .execute(request(
            "INSERT INTO cockpit_types (id, source_value, location) VALUES (1, 41, ST_GeomFromText('POINT(1 2)', 4326))",
            true,
            100,
            0,
        ))
        .await
        .unwrap();
    let types = session
        .execute(request(
            "SELECT computed_value, location FROM cockpit_types",
            false,
            10,
            0,
        ))
        .await
        .unwrap();
    assert_eq!(types.rows[0][0], CellValue::Signed("42".into()));
    let CellValue::Geometry { wkb_base64, srid } = &types.rows[0][1] else {
        panic!("geometry value was not decoded as geometry");
    };
    assert_eq!(*srid, Some(4326));
    let wkb = BASE64_STANDARD.decode(wkb_base64).unwrap();
    assert_eq!(
        wkb.len(),
        21,
        "MySQL's internal SRID prefix must be removed"
    );
    assert!(matches!(wkb[0], 0 | 1));

    let timeout_id = Uuid::new_v4();
    let mut timeout_request = request("SELECT SLEEP(2)", true, 10, 0);
    timeout_request.execution_id = timeout_id;
    timeout_request.timeout_secs = Some(1);
    assert!(matches!(
        session.execute(timeout_request).await,
        Err(CockpitError::Timeout)
    ));
    assert!(
        !session.cancel(timeout_id).await.unwrap(),
        "timed-out query must leave no running entry"
    );
    let after_timeout = session
        .execute(request("SELECT 1", false, 10, 0))
        .await
        .unwrap();
    assert_eq!(after_timeout.rows[0][0], CellValue::Signed("1".into()));

    let original_time_zone = session
        .execute(request("SELECT @@SESSION.time_zone", false, 10, 0))
        .await
        .unwrap()
        .rows[0][0]
        .clone();
    session.begin_read_transaction().await.unwrap();
    let snapshot_detail = session
        .table_detail("cockpit_test", "cockpit_types")
        .await
        .unwrap();
    assert_eq!(snapshot_detail.table.name, "cockpit_types");
    let snapshot_time_zone = session
        .execute(request("SELECT @@SESSION.time_zone", false, 10, 0))
        .await
        .unwrap();
    assert_eq!(
        snapshot_time_zone.rows[0][0],
        CellValue::Text("+00:00".into())
    );
    session.rollback_transaction().await.unwrap();
    let restored_time_zone = session
        .execute(request("SELECT @@SESSION.time_zone", false, 10, 0))
        .await
        .unwrap();
    assert_eq!(restored_time_zone.rows[0][0], original_time_zone);

    session.begin_transaction().await.unwrap();
    session
        .mutate_row(RowMutationRequest {
            database: "cockpit_test".into(),
            table: "cockpit_matrix".into(),
            kind: RowMutationKind::Insert,
            values: vec![("note".into(), CellValue::Text("rolled back".into()))],
            key_values: vec![],
            original_values: vec![],
        })
        .await
        .unwrap();
    assert!(session.transaction_active().await);
    session.rollback_transaction().await.unwrap();

    let after_rollback = session
        .execute(request("SELECT COUNT(*) FROM cockpit_matrix", false, 10, 0))
        .await
        .unwrap();
    assert_eq!(after_rollback.rows[0][0], CellValue::Signed("3".into()));

    session
        .execute(request("DROP PROCEDURE cockpit_matrix_count", true, 100, 0))
        .await
        .unwrap();
    session
        .execute(request("DROP VIEW cockpit_matrix_view", true, 100, 0))
        .await
        .unwrap();
    session
        .execute(request("DROP TABLE cockpit_matrix", true, 100, 0))
        .await
        .unwrap();
    session
        .execute(request("DROP TABLE cockpit_types", true, 100, 0))
        .await
        .unwrap();
    session.close().await.unwrap();

    let mut read_only_profile = test_profile();
    read_only_profile.read_only = true;
    let read_only_session = MySqlDriver
        .open(
            read_only_profile,
            std::env::var("COCKPIT_TEST_MYSQL_PASSWORD").unwrap_or_else(|_| "root".into()),
        )
        .await
        .unwrap();
    read_only_session.begin_read_transaction().await.unwrap();
    assert!(read_only_session.transaction_active().await);
    read_only_session.rollback_transaction().await.unwrap();
    read_only_session.close().await.unwrap();
}
