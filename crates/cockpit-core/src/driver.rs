use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    CellValue, ColumnInfo, ConnectionInfo, ConnectionProfile, DatabaseInfo,
    DatabaseObjectDefinition, DatabaseObjectKind, EventInfo, ExecuteQueryRequest,
    ImportConflictPolicy, QueryResultPage, Result, RoutineInfo, RoutineParameter,
    RowMutationRequest, RowMutationResult, ServerLockInfo, ServerMetric, ServerProcessInfo,
    ServerVariable, TableDetail, TableInfo, TriggerInfo, UserAccount,
};

#[async_trait]
pub trait DatabaseDriver: Send + Sync {
    fn kind(&self) -> &'static str;
    async fn test(&self, profile: &ConnectionProfile, password: &str) -> Result<ConnectionInfo>;
    async fn open(
        &self,
        profile: ConnectionProfile,
        password: String,
    ) -> Result<Arc<dyn DriverSession>>;
}

#[async_trait]
pub trait DriverSession: Send + Sync {
    fn connection_id(&self) -> uuid::Uuid;
    async fn connection_info(&self) -> Result<ConnectionInfo>;
    async fn list_databases(&self) -> Result<Vec<DatabaseInfo>>;
    async fn list_tables(
        &self,
        database: &str,
        filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TableInfo>>;
    async fn list_columns(&self, database: &str, table: &str) -> Result<Vec<ColumnInfo>>;
    async fn table_detail(&self, database: &str, table: &str) -> Result<TableDetail>;
    async fn list_routines(&self, database: &str) -> Result<Vec<RoutineInfo>>;
    async fn list_triggers(&self, database: &str) -> Result<Vec<TriggerInfo>>;
    async fn list_events(&self, database: &str) -> Result<Vec<EventInfo>>;
    async fn object_definition(
        &self,
        database: &str,
        kind: DatabaseObjectKind,
        name: &str,
    ) -> Result<DatabaseObjectDefinition>;
    async fn routine_parameters(&self, database: &str, name: &str)
    -> Result<Vec<RoutineParameter>>;
    async fn list_processes(&self) -> Result<Vec<ServerProcessInfo>>;
    async fn kill_process(&self, process_id: u64) -> Result<()>;
    async fn server_status(&self) -> Result<Vec<ServerMetric>>;
    async fn server_variables(&self, _filter: Option<&str>) -> Result<Vec<ServerVariable>> {
        Err(crate::CockpitError::Unsupported(
            "当前数据库驱动不支持服务器变量".into(),
        ))
    }
    async fn server_locks(&self) -> Result<Vec<ServerLockInfo>> {
        Err(crate::CockpitError::Unsupported(
            "当前数据库驱动不支持锁等待监控".into(),
        ))
    }
    async fn list_users(&self) -> Result<Vec<UserAccount>>;
    async fn user_grants(&self, user: &str, host: &str) -> Result<Vec<String>>;
    async fn execute(&self, request: ExecuteQueryRequest) -> Result<QueryResultPage>;
    async fn mutate_row(&self, request: RowMutationRequest) -> Result<RowMutationResult>;
    async fn insert_rows(
        &self,
        database: &str,
        table: &str,
        columns: &[String],
        rows: &[Vec<CellValue>],
    ) -> Result<u64>;
    async fn insert_rows_with_policy(
        &self,
        database: &str,
        table: &str,
        columns: &[String],
        rows: &[Vec<CellValue>],
        policy: ImportConflictPolicy,
    ) -> Result<u64> {
        if policy != ImportConflictPolicy::Error {
            return Err(crate::CockpitError::Unsupported(
                "当前数据库驱动不支持所选导入冲突策略".into(),
            ));
        }
        self.insert_rows(database, table, columns, rows).await
    }
    async fn begin_transaction(&self) -> Result<()>;
    async fn begin_read_transaction(&self) -> Result<()> {
        self.begin_transaction().await
    }
    async fn commit_transaction(&self) -> Result<()>;
    async fn rollback_transaction(&self) -> Result<()>;
    async fn transaction_active(&self) -> bool;
    async fn cancel(&self, execution_id: uuid::Uuid) -> Result<bool>;
    async fn close(&self) -> Result<()>;
}
