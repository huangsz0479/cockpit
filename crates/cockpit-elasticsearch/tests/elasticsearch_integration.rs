//! 需要 Elasticsearch 服务的集成测试。
//! 通过 COCKPIT_TEST_ES_* 环境变量启用（CI 中由 elasticsearch 服务容器提供）：
//! COCKPIT_TEST_ES_HOST（默认 127.0.0.1）、COCKPIT_TEST_ES_PORT（默认 9200）、
//! COCKPIT_TEST_ES_USER / COCKPIT_TEST_ES_PASSWORD（可选）。

use std::time::Duration;

use chrono::Utc;
use cockpit_core::{
    CellValue, ConnectionProfile, DatabaseDriver, DriverSession, ExecuteQueryRequest, TlsOptions,
};
use cockpit_elasticsearch::ElasticsearchDriver;
use serde_json::{Value, json};
use uuid::Uuid;

fn es_host() -> String {
    std::env::var("COCKPIT_TEST_ES_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

fn es_port() -> u16 {
    std::env::var("COCKPIT_TEST_ES_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(9200)
}

fn es_user() -> String {
    std::env::var("COCKPIT_TEST_ES_USER").unwrap_or_default()
}

fn es_password() -> String {
    std::env::var("COCKPIT_TEST_ES_PASSWORD").unwrap_or_default()
}

fn test_profile() -> ConnectionProfile {
    let now = Utc::now();
    ConnectionProfile {
        id: Uuid::new_v4(),
        driver_kind: cockpit_core::DatabaseKind::Elasticsearch,
        group: None,
        name: "Elasticsearch integration".into(),
        host: es_host(),
        port: es_port(),
        username: es_user(),
        database: None,
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

async fn es_request(method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
    let mut request = reqwest::Client::new()
        .request(
            method,
            format!("http://{}:{}{}", es_host(), es_port(), path),
        )
        .timeout(Duration::from_secs(30));
    let user = es_user();
    if !user.is_empty() {
        request = request.basic_auth(user, Some(es_password()));
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.expect("ES 请求失败");
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    assert!(status.is_success(), "ES 请求 {path} 失败：{status} {body}");
    body
}

/// 准备一个包含 3 篇文档的测试索引，返回索引名。
async fn setup_index(session: &dyn DriverSession, database: &str) -> String {
    let index = format!("cockpit_it_{}", Uuid::new_v4().simple());
    es_request(
        reqwest::Method::PUT,
        &format!("/{index}"),
        Some(json!({
            "mappings": {
                "properties": {
                    "title": { "type": "keyword" },
                    "price": { "type": "integer" },
                    "tags": { "type": "keyword" },
                    "seller": { "properties": { "name": { "type": "text" } } }
                }
            }
        })),
    )
    .await;
    for (title, price) in [("alpha", 1), ("beta", 2), ("gamma", 3)] {
        es_request(
            reqwest::Method::PUT,
            &format!("/{index}/_doc/{title}"),
            Some(json!({
                "title": title,
                "price": price,
                "tags": [format!("t-{title}"), "all"],
                "seller": { "name": format!("shop-{title}") }
            })),
        )
        .await;
    }
    es_request(reqwest::Method::POST, "/_refresh", None).await;
    for _ in 0..30 {
        let tables = session.list_tables(database, None, 100, 0).await.unwrap();
        if tables.iter().any(|table| table.name == index) {
            return index;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("索引 {index} 未在超时前出现");
}

fn query(sql: &str) -> ExecuteQueryRequest {
    ExecuteQueryRequest {
        execution_id: Uuid::new_v4(),
        sql: sql.into(),
        database: None,
        timeout_secs: Some(30),
        allow_write: false,
        page_size: 2,
        row_offset: 0,
    }
}

#[tokio::test]
#[ignore = "requires COCKPIT_TEST_ES_* or the CI Elasticsearch service"]
async fn browsing_sql_paging_and_read_only_guards_work() {
    let profile = test_profile();
    let password = es_password();
    let info = ElasticsearchDriver.test(&profile, &password).await.unwrap();
    assert!(!info.server_version.is_empty());

    let session = ElasticsearchDriver
        .open(profile, password)
        .await
        .expect("connect to Elasticsearch");

    let databases = session.list_databases().await.unwrap();
    assert_eq!(databases.len(), 1);

    let index = setup_index(session.as_ref(), &databases[0].name).await;
    let tables = session
        .list_tables(&databases[0].name, None, 100, 0)
        .await
        .unwrap();
    let entry = tables.iter().find(|table| table.name == index).unwrap();
    // 所属库必须回填集群伪库名，否则前端会误判“表不在当前库”而折叠导航树
    assert_eq!(entry.database, databases[0].name);
    assert_eq!(entry.estimated_rows, Some(3));

    let columns = session.list_columns("", &index).await.unwrap();
    let names: Vec<&str> = columns.iter().map(|column| column.name.as_str()).collect();
    assert!(names.contains(&"title"));
    assert!(names.contains(&"price"));
    assert!(names.contains(&"seller.name"));

    let detail = session.table_detail("", &index).await.unwrap();
    assert!(detail.ddl.contains("\"mappings\""));

    // 分页：page_size=2，第一页 2 行且有更多
    let first = session
        .execute(query(&format!(
            "SELECT title FROM \"{index}\" ORDER BY title"
        )))
        .await
        .unwrap();
    assert_eq!(first.rows.len(), 2);
    assert!(first.has_more);
    assert_eq!(first.rows[0][0], CellValue::Text("alpha".into()));

    // 第二页：顺序翻页走游标快路径
    let second = session
        .execute(ExecuteQueryRequest {
            row_offset: 2,
            ..query(&format!("SELECT title FROM \"{index}\" ORDER BY title"))
        })
        .await
        .unwrap();
    assert_eq!(second.rows.len(), 1);
    assert!(!second.has_more);
    assert_eq!(second.rows[0][0], CellValue::Text("gamma".into()));

    // SELECT *：translate + _search 路径完整取回数组 tags 与嵌套 seller，
    // 并以 _id 作为首列（详细断言见 search_path_returns_document_ids_and_nested_fields）
    let star_page = session
        .execute(query(&format!("SELECT * FROM \"{index}\" ORDER BY title")))
        .await
        .unwrap();
    assert_eq!(star_page.columns[0].name, "_id");
    assert!(star_page.columns.iter().any(|column| column.name == "tags"));
    assert!(
        star_page
            .columns
            .iter()
            .any(|column| column.name == "seller")
    );
    assert!(
        star_page
            .columns
            .iter()
            .any(|column| column.name == "title")
    );

    // ES SQL 支持 LIMIT（无 OFFSET），且容忍尾部分号（驱动会剥掉）
    let limit_page = session
        .execute(query(&format!(
            "SELECT title FROM \"{index}\" ORDER BY title LIMIT 2;"
        )))
        .await
        .unwrap();
    assert_eq!(limit_page.rows.len(), 2);
    assert_eq!(limit_page.rows[0][0], CellValue::Text("alpha".into()));

    // 只读边界：写入类操作必须被拒绝
    let mutation = session
        .mutate_row(cockpit_core::RowMutationRequest {
            database: String::new(),
            table: index.clone(),
            kind: cockpit_core::RowMutationKind::Delete,
            values: Vec::new(),
            key_values: vec![("_id".into(), CellValue::Text("alpha".into()))],
            original_values: Vec::new(),
        })
        .await;
    assert!(mutation.is_err());
    assert!(session.insert_rows("", &index, &[], &[]).await.is_err());
    assert!(!session.transaction_active().await);

    cleanup_index(&index).await;
}

#[tokio::test]
#[ignore = "requires COCKPIT_TEST_ES_* or the CI Elasticsearch service"]
async fn search_path_returns_document_ids_and_nested_fields() {
    let profile = test_profile();
    let password = es_password();
    let session = ElasticsearchDriver
        .open(profile, password)
        .await
        .expect("connect to Elasticsearch");
    let databases = session.list_databases().await.unwrap();
    let index = setup_index(session.as_ref(), &databases[0].name).await;
    let browse = |sql: String, row_offset: usize| ExecuteQueryRequest {
        row_offset,
        page_size: 10,
        ..query(&sql)
    };

    let page = session
        .execute(browse(
            format!("SELECT * FROM \"{index}\" ORDER BY title"),
            0,
        ))
        .await
        .unwrap();
    // _id 打头，数组 tags 与嵌套 seller 以 JSON 列完整呈现
    assert_eq!(page.columns[0].name, "_id");
    let column_index = |name: &str| {
        page.columns
            .iter()
            .position(|column| column.name == name)
            .unwrap()
    };
    assert_eq!(page.rows.len(), 3);
    assert!(!page.has_more);
    assert_eq!(page.rows[0][0], CellValue::Text("alpha".into()));
    assert_eq!(page.rows[1][0], CellValue::Text("beta".into()));
    assert_eq!(page.rows[2][0], CellValue::Text("gamma".into()));
    // 刚写入的文档修改次数为 1
    let version = column_index("_version");
    assert_eq!(page.rows[0][version], CellValue::Signed("1".into()));
    let tags = column_index("tags");
    assert_eq!(
        page.rows[0][tags],
        CellValue::Json("[\"t-alpha\",\"all\"]".into())
    );
    let seller = column_index("seller");
    assert_eq!(
        page.rows[0][seller],
        CellValue::Json("{\"name\":\"shop-alpha\"}".into())
    );

    // 翻页：from+size 无游标，偏移 1 起取剩余两行
    let second = session
        .execute(browse(
            format!("SELECT * FROM \"{index}\" ORDER BY title"),
            1,
        ))
        .await
        .unwrap();
    assert_eq!(second.rows.len(), 2);
    assert!(!second.has_more);
    assert_eq!(second.rows[0][0], CellValue::Text("beta".into()));

    // LIMIT 生效：不因多取一行判定 has_more 而越过上限
    let limited = session
        .execute(query(&format!("SELECT * FROM \"{index}\" LIMIT 2")))
        .await
        .unwrap();
    assert_eq!(limited.rows.len(), 2);
    assert!(!limited.has_more);

    // WHERE 条件随 translate 一并生效
    let filtered = session
        .execute(query(&format!("SELECT * FROM \"{index}\" WHERE price = 2")))
        .await
        .unwrap();
    assert_eq!(filtered.rows.len(), 1);
    assert_eq!(filtered.rows[0][0], CellValue::Text("beta".into()));

    // 聚合查询不适用 _search 路径，回退 ES SQL，无 _id 列
    let aggregate = session
        .execute(query(&format!("SELECT count(*) FROM \"{index}\"")))
        .await
        .unwrap();
    assert!(!aggregate.columns.iter().any(|column| column.name == "_id"));
    assert_eq!(aggregate.rows[0][0], CellValue::Signed("3".into()));

    cleanup_index(&index).await;
}

#[tokio::test]
#[ignore = "requires COCKPIT_TEST_ES_* or the CI Elasticsearch service"]
async fn update_document_replaces_source_by_id() {
    let profile = test_profile();
    let password = es_password();
    let session = ElasticsearchDriver
        .open(profile, password)
        .await
        .expect("connect to Elasticsearch");
    let databases = session.list_databases().await.unwrap();
    let index = setup_index(session.as_ref(), &databases[0].name).await;

    // 整文档替换：编辑后的 JSON 按 _id 写回，重新查询立即可见（驱动保存后刷新索引）
    session
        .update_document(
            &index,
            "alpha",
            &json!({
                "title": "alpha-edited",
                "price": 99,
                "tags": ["edited"],
                "seller": { "name": "shop-alpha" }
            }),
        )
        .await
        .unwrap();
    let page = session
        .execute(ExecuteQueryRequest {
            page_size: 10,
            ..query(&format!("SELECT * FROM \"{index}\""))
        })
        .await
        .unwrap();
    assert_eq!(page.source_table.as_deref(), Some(index.as_str()));
    let alpha = page
        .rows
        .iter()
        .find(|row| matches!(row.first(), Some(CellValue::Text(id)) if id == "alpha"))
        .expect("alpha 文档仍存在");
    let title = page
        .columns
        .iter()
        .position(|column| column.name == "title")
        .unwrap();
    assert_eq!(alpha[title], CellValue::Text("alpha-edited".into()));
    // 整文档替换一次后修改计数递增为 2
    let version = page
        .columns
        .iter()
        .position(|column| column.name == "_version")
        .unwrap();
    assert_eq!(alpha[version], CellValue::Signed("2".into()));
    let tags = page
        .columns
        .iter()
        .position(|column| column.name == "tags")
        .unwrap();
    assert_eq!(alpha[tags], CellValue::Json("[\"edited\"]".into()));

    // 只读连接拒绝写入
    let mut read_only_profile = test_profile();
    read_only_profile.read_only = true;
    let read_only = ElasticsearchDriver
        .open(read_only_profile, es_password())
        .await
        .unwrap();
    let denied = read_only
        .update_document(&index, "alpha", &json!({ "title": "nope" }))
        .await;
    assert!(denied.is_err());

    // 非法索引名直接拒绝，不发起请求
    assert!(
        session
            .update_document("../_all", "x", &json!({}))
            .await
            .is_err()
    );
    assert!(
        session
            .update_document(&index, "", &json!({}))
            .await
            .is_err()
    );

    cleanup_index(&index).await;
}

#[tokio::test]
#[ignore = "requires COCKPIT_TEST_ES_* or the CI Elasticsearch service"]
async fn index_management_creates_clears_and_deletes_indices() {
    let profile = test_profile();
    let password = es_password();
    let session = ElasticsearchDriver
        .open(profile, password)
        .await
        .expect("connect to Elasticsearch");
    let databases = session.list_databases().await.unwrap();
    let index = format!("cockpit_it_{}", Uuid::new_v4().simple());

    // 创建索引：带 mappings 时字段立即可见
    session
        .create_index(
            &index,
            Some(&json!({
                "mappings": { "properties": { "title": { "type": "keyword" } } }
            })),
        )
        .await
        .unwrap();
    let columns = session.list_columns("", &index).await.unwrap();
    assert_eq!(columns.len(), 1);
    assert_eq!(columns[0].name, "title");

    // 写入 3 篇文档后清空：文档清零、索引与 mapping 保留
    for (title, price) in [("alpha", 1), ("beta", 2), ("gamma", 3)] {
        es_request(
            reqwest::Method::PUT,
            &format!("/{index}/_doc/{title}"),
            Some(json!({ "title": title, "price": price })),
        )
        .await;
    }
    es_request(reqwest::Method::POST, "/_refresh", None).await;
    let deleted = session.clear_index(&index).await.unwrap();
    assert_eq!(deleted, 3);
    es_request(reqwest::Method::POST, "/_refresh", None).await;
    let count = session
        .execute(query(&format!("SELECT count(*) FROM \"{index}\"")))
        .await
        .unwrap();
    assert_eq!(count.rows[0][0], CellValue::Signed("0".into()));
    let columns_after = session.list_columns("", &index).await.unwrap();
    assert_eq!(columns_after.len(), 1);

    // 删除索引后从列表消失，重复删除报索引不存在
    session.delete_index(&index).await.unwrap();
    let tables = session
        .list_tables(&databases[0].name, Some(&index), 100, 0)
        .await
        .unwrap();
    assert!(tables.is_empty());
    let again = session.delete_index(&index).await;
    assert!(again.is_err());

    // 空索引名创建、非法索引名删除在本地直接拒绝
    assert!(session.create_index("", None).await.is_err());
    assert!(session.delete_index("../_all").await.is_err());
    assert!(session.clear_index("Bad Name").await.is_err());

    // 只读连接拒绝全部索引管理写操作
    let mut read_only_profile = test_profile();
    read_only_profile.read_only = true;
    let read_only = ElasticsearchDriver
        .open(read_only_profile, es_password())
        .await
        .unwrap();
    assert!(
        read_only
            .create_index("cockpit_it_denied", None)
            .await
            .is_err()
    );
    assert!(read_only.clear_index(&index).await.is_err());
    assert!(read_only.delete_index(&index).await.is_err());
}

async fn cleanup_index(index: &str) {
    let _ = es_request(reqwest::Method::DELETE, &format!("/{index}"), None).await;
}
