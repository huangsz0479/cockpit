use chrono::Utc;
use cockpit_core::{
    CellValue, CockpitError, ConnectionProfile, DatabaseDriver, ExecuteQueryRequest,
    RowMutationKind, RowMutationRequest, TlsOptions,
};
use cockpit_postgres::PostgresDriver;
use uuid::Uuid;

fn test_profile() -> ConnectionProfile {
    let now = Utc::now();
    ConnectionProfile {
        id: Uuid::new_v4(),
        driver_kind: cockpit_core::DatabaseKind::PostgreSql,
        group: None,
        name: "PostgreSQL integration".into(),
        host: std::env::var("COCKPIT_TEST_POSTGRES_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        port: std::env::var("COCKPIT_TEST_POSTGRES_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(5432),
        username: std::env::var("COCKPIT_TEST_POSTGRES_USER").unwrap_or_else(|_| "postgres".into()),
        database: Some(
            std::env::var("COCKPIT_TEST_POSTGRES_DATABASE").unwrap_or_else(|_| "postgres".into()),
        ),
        tls: TlsOptions::default(),
        ssh: None,
        connect_timeout_secs: 10,
        query_timeout_secs: 30,
        pool_size: 1,
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
        database: Some("public".into()),
        timeout_secs: Some(30),
        allow_write,
        page_size,
        row_offset,
    }
}

fn text(value: &str) -> CellValue {
    CellValue::Text(value.into())
}

#[tokio::test]
#[ignore = "requires COCKPIT_TEST_POSTGRES_* or the CI PostgreSQL service"]
async fn streaming_paging_read_only_and_timeout_work() {
    let profile = test_profile();
    let password =
        std::env::var("COCKPIT_TEST_POSTGRES_PASSWORD").unwrap_or_else(|_| "postgres".into());
    let session = PostgresDriver
        .open(profile, password.clone())
        .await
        .expect("connect to PostgreSQL");

    let version = session
        .execute(request("SELECT VERSION()", false, 10, 0))
        .await
        .unwrap();
    assert!(!version.rows.is_empty());

    session
        .execute(request("DROP TABLE IF EXISTS cockpit_matrix", true, 100, 0))
        .await
        .unwrap();
    session
        .execute(request(
            "CREATE TABLE cockpit_matrix (id BIGSERIAL NOT NULL PRIMARY KEY, note TEXT NOT NULL)",
            true,
            100,
            0,
        ))
        .await
        .unwrap();

    // 纯 DML 必须上报真实的受影响行数（修复前会误报为 0）。
    let inserted = session
        .execute(request(
            "INSERT INTO cockpit_matrix (note) VALUES ('one'),('two'),('three'),('four'),('five')",
            true,
            100,
            0,
        ))
        .await
        .unwrap();
    assert_eq!(inserted.affected_rows, 5);

    // 流式分页：跳过 row_offset 行、最多收集 page_size 行，并正确判断 has_more。
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
    assert_eq!(first.rows[0], vec![text("1"), text("one")]);
    assert_eq!(first.rows[1], vec![text("2"), text("two")]);
    assert!(first.has_more);
    // SELECT 的 affected_rows 应为 0，而不是返回行数。
    assert_eq!(first.affected_rows, 0);

    let second = session
        .execute(request(
            "SELECT id, note FROM cockpit_matrix ORDER BY id",
            false,
            2,
            2,
        ))
        .await
        .unwrap();
    assert_eq!(second.rows.len(), 2);
    assert_eq!(second.rows[0], vec![text("3"), text("three")]);
    assert!(second.has_more);

    let last = session
        .execute(request(
            "SELECT id, note FROM cockpit_matrix ORDER BY id",
            false,
            2,
            4,
        ))
        .await
        .unwrap();
    assert_eq!(last.rows.len(), 1);
    assert_eq!(last.rows[0], vec![text("5"), text("five")]);
    assert!(!last.has_more);

    // 多条语句仍返回多个结果集（流式 simple query 保留多语句语义）。
    let multi = session
        .execute(request("SELECT 1 AS a; SELECT 2 AS b", false, 10, 0))
        .await
        .unwrap();
    assert_eq!(multi.rows, vec![vec![text("1")]]);
    assert_eq!(multi.additional_result_sets.len(), 1);
    assert_eq!(multi.additional_result_sets[0].rows, vec![vec![text("2")]]);

    // 超时后服务端查询被真正取消，连接仍可复用。
    // 用 pg_sleep(30)：若取消机制失效，排空会等到清理超时（10s）并断开连接，
    // 后续 SELECT 1 会失败，测试会响亮地暴露问题。
    let mut timeout_request = request("SELECT pg_sleep(30)", false, 10, 0);
    timeout_request.timeout_secs = Some(1);
    assert!(matches!(
        session.execute(timeout_request).await,
        Err(CockpitError::Timeout)
    ));
    let after_timeout = session
        .execute(request("SELECT 1", false, 10, 0))
        .await
        .unwrap();
    assert_eq!(after_timeout.rows[0][0], text("1"));

    // 只读连接拒绝所有写路径入口，但仍可执行读查询与只读事务。
    let mut read_only_profile = test_profile();
    read_only_profile.read_only = true;
    let read_only_session = PostgresDriver
        .open(read_only_profile, password)
        .await
        .unwrap();
    read_only_session
        .execute(request("SELECT 1", false, 10, 0))
        .await
        .unwrap();
    assert!(matches!(
        read_only_session
            .execute(request(
                "INSERT INTO cockpit_matrix (note) VALUES ('x')",
                true,
                10,
                0,
            ))
            .await,
        Err(CockpitError::Query(_))
    ));
    assert!(
        read_only_session
            .mutate_row(RowMutationRequest {
                database: "public".into(),
                table: "cockpit_matrix".into(),
                kind: RowMutationKind::Insert,
                values: vec![("note".into(), CellValue::Text("x".into()))],
                key_values: vec![],
                original_values: vec![],
            })
            .await
            .is_err()
    );
    assert!(
        read_only_session
            .insert_rows(
                "public",
                "cockpit_matrix",
                &["note".into()],
                &[vec![CellValue::Text("x".into())]],
            )
            .await
            .is_err()
    );
    assert!(read_only_session.begin_transaction().await.is_err());
    read_only_session.begin_read_transaction().await.unwrap();
    assert!(read_only_session.transaction_active().await);
    read_only_session.rollback_transaction().await.unwrap();
    read_only_session.close().await.unwrap();

    session
        .execute(request("DROP TABLE cockpit_matrix", true, 100, 0))
        .await
        .unwrap();
    session.close().await.unwrap();
}
