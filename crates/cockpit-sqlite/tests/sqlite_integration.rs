use chrono::Utc;
use cockpit_core::{
    CellValue, ConnectionProfile, DatabaseDriver, DatabaseObjectKind, ExecuteQueryRequest,
    ImportConflictPolicy, RowMutationKind, RowMutationRequest, TlsOptions,
};
use cockpit_sqlite::SqliteDriver;
use uuid::Uuid;

fn profile() -> ConnectionProfile {
    let now = Utc::now();
    ConnectionProfile {
        id: Uuid::new_v4(),
        driver_kind: cockpit_core::DatabaseKind::Sqlite,
        group: None,
        name: "memory".into(),
        host: ":memory:".into(),
        port: 1,
        username: String::new(),
        database: Some("main".into()),
        tls: TlsOptions::default(),
        ssh: None,
        connect_timeout_secs: 5,
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
        database: Some("main".into()),
        timeout_secs: Some(30),
        allow_write,
        page_size,
        row_offset,
    }
}

#[tokio::test]
async fn pagination_skips_offset_and_reports_has_more() {
    let session = SqliteDriver.open(profile(), String::new()).await.unwrap();
    session
        .execute(request(
            "CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            true,
            100,
            0,
        ))
        .await
        .unwrap();

    let mut values = Vec::new();
    for id in 1..=250 {
        values.push(format!("({id}, 'item-{id}')"));
    }
    session
        .execute(request(
            format!("INSERT INTO items(id, name) VALUES {}", values.join(", ")),
            true,
            100,
            0,
        ))
        .await
        .unwrap();

    // 第 1 页：0 偏移，100 行，还有更多
    let first = session
        .execute(request(
            "SELECT id, name FROM items ORDER BY id",
            false,
            100,
            0,
        ))
        .await
        .unwrap();
    assert_eq!(first.rows.len(), 100);
    assert!(first.has_more);
    assert_eq!(first.rows[0][0], CellValue::Signed("1".into()));
    assert_eq!(first.rows[99][0], CellValue::Signed("100".into()));

    // 第 2 页：偏移 100，100 行，还有更多
    let second = session
        .execute(request(
            "SELECT id, name FROM items ORDER BY id",
            false,
            100,
            100,
        ))
        .await
        .unwrap();
    assert_eq!(second.rows.len(), 100);
    assert!(second.has_more);
    assert_eq!(second.rows[0][0], CellValue::Signed("101".into()));
    assert_eq!(second.rows[99][0], CellValue::Signed("200".into()));

    // 第 3 页：偏移 200，50 行，没有更多
    let third = session
        .execute(request(
            "SELECT id, name FROM items ORDER BY id",
            false,
            100,
            200,
        ))
        .await
        .unwrap();
    assert_eq!(third.rows.len(), 50);
    assert!(!third.has_more);
    assert_eq!(third.rows[0][0], CellValue::Signed("201".into()));
    assert_eq!(third.rows[49][0], CellValue::Signed("250".into()));

    // 超出范围的偏移：空结果且 has_more 为 false
    let beyond = session
        .execute(request(
            "SELECT id FROM items ORDER BY id",
            false,
            100,
            1_000,
        ))
        .await
        .unwrap();
    assert!(beyond.rows.is_empty());
    assert!(!beyond.has_more);

    // page_size 下限钳制为 1
    let clamped = session
        .execute(request("SELECT id FROM items ORDER BY id", false, 0, 0))
        .await
        .unwrap();
    assert_eq!(clamped.rows.len(), 1);
    assert!(clamped.has_more);
    assert_eq!(clamped.page_size, 1);

    // 事务内的分页行为保持一致
    session.begin_transaction().await.unwrap();
    let in_transaction = session
        .execute(request("SELECT id FROM items ORDER BY id", false, 100, 100))
        .await
        .unwrap();
    assert_eq!(in_transaction.rows.len(), 100);
    assert!(in_transaction.has_more);
    assert_eq!(in_transaction.rows[0][0], CellValue::Signed("101".into()));
    session.rollback_transaction().await.unwrap();

    session.close().await.unwrap();
}

#[tokio::test]
async fn read_only_connection_rejects_writes() {
    let path = std::env::temp_dir().join(format!("cockpit-sqlite-readonly-{}.db", Uuid::new_v4()));

    // 先用可写连接准备数据（:memory: 是每连接独立的，必须用同一个文件）
    let mut setup_profile = profile();
    setup_profile.host = path.display().to_string();
    let setup = SqliteDriver
        .open(setup_profile, String::new())
        .await
        .unwrap();
    setup
        .execute(request(
            "CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            true,
            100,
            0,
        ))
        .await
        .unwrap();
    setup
        .execute(request(
            "INSERT INTO items(id, name) VALUES (1, 'a'), (2, 'b')",
            true,
            100,
            0,
        ))
        .await
        .unwrap();
    setup.close().await.unwrap();

    let mut read_only_profile = profile();
    read_only_profile.host = path.display().to_string();
    read_only_profile.read_only = true;
    let session = SqliteDriver
        .open(read_only_profile, String::new())
        .await
        .unwrap();

    // 读操作仍然可用
    let page = session
        .execute(request(
            "SELECT id, name FROM items ORDER BY id",
            false,
            100,
            0,
        ))
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 2);

    // 行更新被拒绝
    let error = session
        .mutate_row(RowMutationRequest {
            database: "main".into(),
            table: "items".into(),
            kind: RowMutationKind::Update,
            values: vec![("name".into(), CellValue::Text("x".into()))],
            key_values: vec![("id".into(), CellValue::Signed("1".into()))],
            original_values: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("只读"),
        "unexpected error: {error:?}"
    );

    // 行删除被拒绝
    let error = session
        .mutate_row(RowMutationRequest {
            database: "main".into(),
            table: "items".into(),
            kind: RowMutationKind::Delete,
            values: Vec::new(),
            key_values: vec![("id".into(), CellValue::Signed("1".into()))],
            original_values: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("只读"),
        "unexpected error: {error:?}"
    );

    // 行插入被拒绝
    let error = session
        .mutate_row(RowMutationRequest {
            database: "main".into(),
            table: "items".into(),
            kind: RowMutationKind::Insert,
            values: vec![
                ("id".into(), CellValue::Signed("3".into())),
                ("name".into(), CellValue::Text("c".into())),
            ],
            key_values: Vec::new(),
            original_values: Vec::new(),
        })
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("只读"),
        "unexpected error: {error:?}"
    );

    // 批量导入被拒绝
    let error = session
        .insert_rows_with_policy(
            "main",
            "items",
            &["id".into(), "name".into()],
            &[vec![
                CellValue::Signed("4".into()),
                CellValue::Text("d".into()),
            ]],
            ImportConflictPolicy::Error,
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("只读"),
        "unexpected error: {error:?}"
    );

    // 通过 execute 的 DDL / DML 也被拒绝
    let error = session
        .execute(request("DROP TABLE items", true, 100, 0))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("只读"),
        "unexpected error: {error:?}"
    );
    let error = session
        .execute(request(
            "INSERT INTO items(id, name) VALUES (5, 'e')",
            true,
            100,
            0,
        ))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("只读"),
        "unexpected error: {error:?}"
    );

    // 开启写事务被拒绝
    let error = session.begin_transaction().await.unwrap_err();
    assert!(
        error.to_string().contains("只读"),
        "unexpected error: {error:?}"
    );

    // 数据未被改动
    let count = session
        .execute(request("SELECT COUNT(*) FROM items", false, 100, 0))
        .await
        .unwrap();
    assert_eq!(count.rows[0][0], CellValue::Signed("2".into()));

    session.close().await.unwrap();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

#[tokio::test]
async fn crud_and_transaction_semantics_are_preserved() {
    let session = SqliteDriver.open(profile(), String::new()).await.unwrap();
    session
        .execute(request(
            "CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            true,
            100,
            0,
        ))
        .await
        .unwrap();

    // 基本元数据路径
    let info = session.connection_info().await.unwrap();
    assert!(!info.server_version.is_empty());
    let databases = session.list_databases().await.unwrap();
    assert!(databases.iter().any(|item| item.name == "main"));
    let tables = session
        .list_tables("main", Some("items"), 20, 0)
        .await
        .unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "items");
    let detail = session.table_detail("main", "items").await.unwrap();
    assert_eq!(detail.columns.len(), 2);
    let status = session.server_status().await.unwrap();
    assert!(status.iter().any(|metric| metric.name == "page_count"));

    // 显式索引与复合主键的索引检测
    session
        .execute(request(
            "CREATE INDEX idx_items_name ON items(name)",
            true,
            100,
            0,
        ))
        .await
        .unwrap();
    let indexed_detail = session.table_detail("main", "items").await.unwrap();
    assert!(
        indexed_detail
            .indexes
            .iter()
            .any(|index| index.name == "idx_items_name" && !index.primary)
    );
    session
        .execute(request(
            "CREATE TABLE pair(a TEXT NOT NULL, b TEXT NOT NULL, PRIMARY KEY (a, b))",
            true,
            100,
            0,
        ))
        .await
        .unwrap();
    let pair_detail = session.table_detail("main", "pair").await.unwrap();
    assert!(
        pair_detail.indexes.iter().any(|index| {
            index.primary && index.columns == vec!["a".to_string(), "b".to_string()]
        })
    );

    // mutate_row 插入
    for name in ["one", "two", "three"] {
        session
            .mutate_row(RowMutationRequest {
                database: "main".into(),
                table: "items".into(),
                kind: RowMutationKind::Insert,
                values: vec![("name".into(), CellValue::Text(name.into()))],
                key_values: Vec::new(),
                original_values: Vec::new(),
            })
            .await
            .unwrap();
    }
    let page = session
        .execute(request(
            "SELECT id, name FROM items ORDER BY id",
            false,
            10,
            0,
        ))
        .await
        .unwrap();
    assert_eq!(page.rows.len(), 3);
    assert!(!page.has_more);

    // mutate_row 更新
    let updated = session
        .mutate_row(RowMutationRequest {
            database: "main".into(),
            table: "items".into(),
            kind: RowMutationKind::Update,
            values: vec![("name".into(), CellValue::Text("ONE".into()))],
            key_values: vec![("id".into(), CellValue::Signed("1".into()))],
            original_values: vec![
                ("id".into(), CellValue::Signed("1".into())),
                ("name".into(), CellValue::Text("one".into())),
            ],
        })
        .await
        .unwrap();
    assert_eq!(updated.affected_rows, 1);
    assert!(!updated.concurrent_change);

    // mutate_row 删除
    let deleted = session
        .mutate_row(RowMutationRequest {
            database: "main".into(),
            table: "items".into(),
            kind: RowMutationKind::Delete,
            values: Vec::new(),
            key_values: vec![("id".into(), CellValue::Signed("2".into()))],
            original_values: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(deleted.affected_rows, 1);
    assert!(!deleted.concurrent_change);

    // 批量导入
    let inserted = session
        .insert_rows_with_policy(
            "main",
            "items",
            &["id".into(), "name".into()],
            &[
                vec![
                    CellValue::Signed("4".into()),
                    CellValue::Text("four".into()),
                ],
                vec![
                    CellValue::Signed("5".into()),
                    CellValue::Text("five".into()),
                ],
            ],
            ImportConflictPolicy::Error,
        )
        .await
        .unwrap();
    assert_eq!(inserted, 2);

    // 视图定义
    session
        .execute(request(
            "CREATE VIEW items_view AS SELECT id, name FROM items",
            true,
            100,
            0,
        ))
        .await
        .unwrap();
    let view = session
        .object_definition("main", DatabaseObjectKind::View, "items_view")
        .await
        .unwrap();
    assert!(view.ddl.to_ascii_uppercase().contains("CREATE"));

    // 事务：插入后回滚
    session.begin_transaction().await.unwrap();
    assert!(session.transaction_active().await);
    session
        .mutate_row(RowMutationRequest {
            database: "main".into(),
            table: "items".into(),
            kind: RowMutationKind::Insert,
            values: vec![("name".into(), CellValue::Text("rolled back".into()))],
            key_values: Vec::new(),
            original_values: Vec::new(),
        })
        .await
        .unwrap();
    session.rollback_transaction().await.unwrap();
    assert!(!session.transaction_active().await);
    let count = session
        .execute(request("SELECT COUNT(*) FROM items", false, 10, 0))
        .await
        .unwrap();
    assert_eq!(count.rows[0][0], CellValue::Signed("4".into()));

    // 事务：插入后提交
    session.begin_transaction().await.unwrap();
    session
        .mutate_row(RowMutationRequest {
            database: "main".into(),
            table: "items".into(),
            kind: RowMutationKind::Insert,
            values: vec![("name".into(), CellValue::Text("committed".into()))],
            key_values: Vec::new(),
            original_values: Vec::new(),
        })
        .await
        .unwrap();
    session.commit_transaction().await.unwrap();
    assert!(!session.transaction_active().await);
    let count = session
        .execute(request("SELECT COUNT(*) FROM items", false, 10, 0))
        .await
        .unwrap();
    assert_eq!(count.rows[0][0], CellValue::Signed("5".into()));

    // 重复开启事务被拒绝
    session.begin_transaction().await.unwrap();
    let error = session.begin_transaction().await.unwrap_err();
    assert!(
        error.to_string().contains("活动事务"),
        "unexpected error: {error:?}"
    );
    session.rollback_transaction().await.unwrap();

    // 没有活动事务时提交被拒绝
    let error = session.commit_transaction().await.unwrap_err();
    assert!(
        error.to_string().contains("没有活动事务"),
        "unexpected error: {error:?}"
    );

    // 没有活动事务时回滚是幂等的
    session.rollback_transaction().await.unwrap();

    session.close().await.unwrap();
}
