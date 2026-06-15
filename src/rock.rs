use axum::{http::StatusCode, response::IntoResponse};
use sea_query::{MysqlQueryBuilder, QueryStatementBuilder};
use sea_query_sqlx::{SqlxBinder, SqlxValues};

#[derive(Debug)]
pub struct AppError(anyhow::Error);
impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}
impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

pub type AppResult<T> = anyhow::Result<T, AppError>;

/// 统一的通用高性能所有权守卫器
pub struct SqlxQuery {
    pub sql: String,
    pub values: SqlxValues,
}

pub trait SeaQuerySqlxExt {
    /// 核心破局点：接收 &mut self，适用于所有实现了 sea-query 构建特性的语句（增删改查全覆盖）
    fn to_sqlx(&mut self) -> SqlxQuery;
}

// 统一为所有海量构建器（Select, Insert, Update, Delete）提供通用实现
impl<T> SeaQuerySqlxExt for T
where
    T: QueryStatementBuilder + SqlxBinder + Default,
{
    fn to_sqlx(&mut self) -> SqlxQuery {
        // 调用 sea-query-sqlx 的原生构建方法
        let (sql, values) = self.build_sqlx(MysqlQueryBuilder);

        SqlxQuery { sql, values }
    }
}

/// 智能宏：同时完美支持【查】和【增删改】
#[macro_export]
macro_rules! execute {
    // 模式 A：【增删改】动作。不传类型参数，直接就地展开为 sqlx::query_with，返回原生的 Query
    ($query:expr) => {{
        let $crate::rock::SqlxQuery { sql, values } = $query;
        let safe_sql = sqlx::AssertSqlSafe(sql.as_str());
        sqlx::query_with::<sqlx::MySql, _>(safe_sql, values)
    }};

    // 模式 B：【查】动作。传入目标类型，就地展开为 sqlx::query_as_with，返回原生的 QueryAs
    ($query:expr, $output_type:ty) => {{
        let $crate::rock::SqlxQuery { sql, values } = $query;
        let safe_sql = sqlx::AssertSqlSafe(sql.as_str());
        sqlx::query_as_with::<sqlx::MySql, $output_type, _>(safe_sql, values)
    }};
}
