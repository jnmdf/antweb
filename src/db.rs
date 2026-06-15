use std::time::Duration;

use crate::config::MYSQL_URL;
use anyhow::{Context, Result};
use sqlx::{MySql, Pool, mysql::MySqlPoolOptions};
use tokio::sync::OnceCell;

static GLOBAL_DB: OnceCell<Pool<MySql>> = OnceCell::const_new();

pub async fn db() -> Result<&'static Pool<MySql>> {
    GLOBAL_DB
        .get_or_try_init(|| async {
            MySqlPoolOptions::new()
                .max_connections(10)
                .idle_timeout(Duration::from_secs(30))
                .connect(MYSQL_URL)
                .await
                .context("connect mysql")
        })
        .await
}
