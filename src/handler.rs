use crate::{
    db::db,
    execute,
    rock::{AppResult, SeaQuerySqlxExt},
};
use axum::{
    Json, Router,
    extract::Path,
    routing::{delete, get, post, put},
};
use my_macros::SeaModel;
use sea_query::{Expr, ExprTrait, Query};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
#[derive(Deserialize)]
pub struct CreateUserReq {
    pub nickname: String,
    pub email: String,
}

#[derive(Deserialize)]
pub struct UpdateUserReq {
    pub nickname: Option<String>,
    pub email: Option<String>,
}
pub fn router() -> Router {
    Router::new()
        .route("/user/{id}", get(get_user))
        .route("/users", get(list_user))
        .route("/user", post(create_user))
        .route("/user/put", put(update_user))
        .route("/user/{id}", delete(delete_user))
}
#[derive(Debug, Serialize, Deserialize, FromRow)]
struct User {
    id: i32,
    age: i32,
    name: String,
    email: String,
}
/// Data Transfer Object
#[derive(Debug, Deserialize, SeaModel)]
struct CreateUserDto {
    age: i32,
    name: String,
    email: String,
}
#[derive(Debug, Deserialize, SeaModel)]
struct UpdateUserDto {
    age: Option<i32>,
    #[serde()]
    name: Option<String>,
}
async fn get_user(Path(id): Path<u64>) -> AppResult<Json<Value>> {
    let q = Query::select()
        .column("id")
        .column("age")
        .column("name")
        .column("email")
        .from("users")
        .and_where(Expr::col("id").eq(id))
        .to_sqlx();
    let query = execute!(q, User);
    match query.fetch_optional(db().await?).await? {
        Some(user) => Ok(Json(json!(user))),
        None => Ok(Json(json!({"msg": "Record not found"}))),
    }
}
async fn list_user() -> AppResult<Json<Value>> {
    let q = Query::select()
        .column("id")
        .column("age")
        .column("name")
        .column("email")
        .from("users")
        .and_where(Expr::col("age").lt(100))
        .to_sqlx();
    let query = execute!(q, User);
    let reply = query.fetch_all(db().await?).await?;
    Ok(Json(json!({"data": reply})))
}
#[axum::debug_handler]
// 工业级正统规范 —— 动静分离（DTO 与 Entity 剥离）
async fn create_user(Json(user): Json<CreateUserDto>) -> AppResult<Json<Value>> {
    let q = Query::insert()
        .into_table("users")
        .columns(CreateUserDto::columns_auto())
        .values_panic(user.into_row_values())
        .to_sqlx();
    let query = execute!(q);
    let reply = query.execute(db().await?).await?;
    Ok(Json(json!({"data": reply.rows_affected()})))
}
async fn update_user(Json(user): Json<UpdateUserDto>) -> AppResult<Json<Value>> {
    let q = Query::update()
        .table("users")
        .values(user.into_column_values())
        .and_where(Expr::col("age").lt(100))
        .to_sqlx();
    let query = execute!(q);
    let reply = query.execute(db().await?).await?;
    Ok(Json(json!({"data": reply.rows_affected()})))
}
async fn delete_user(Path(id): Path<u64>) -> AppResult<Json<Value>> {
    let q = Query::delete()
        .from_table("users")
        .and_where(Expr::col("id").eq(id))
        .to_sqlx();
    let query = execute!(q);
    let reply = query.execute(db().await?).await?;
    Ok(Json(json!({"data": reply.rows_affected()})))
}
