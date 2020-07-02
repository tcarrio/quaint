mod config;
mod conversion;
mod error;

use crate::{
    ast::{Query, Value},
    connector::{bind::Bind, metrics, queryable::*, ResultSet},
    error::Error,
    visitor::{self, Visitor},
};
use async_trait::async_trait;
pub use config::*;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteRow},
    Connect, Executor, Row as _, SqliteConnection,
};
use std::{collections::HashSet, convert::TryFrom, future::Future, time::Duration};
use tokio::{sync::Mutex, time::timeout};

/// A connector interface for the SQLite database
pub struct Sqlite {
    pub(crate) connection: Mutex<SqliteConnection>,
    /// This is not a `PathBuf` because we need to `ATTACH` the database to the path, and this can
    /// only be done with UTF-8 paths.
    pub(crate) file_path: String,
    pub(crate) socket_timeout: Option<Duration>,
}

impl Sqlite {
    pub async fn new(file_path: &str) -> crate::Result<Sqlite> {
        let params = SqliteParams::try_from(file_path)?;
        let opts = SqliteConnectOptions::new().statement_cache_capacity(params.statement_cache_size);
        let conn = SqliteConnection::connect_with(&opts).await?;

        let connection = Mutex::new(conn);
        let file_path = params.file_path;
        let socket_timeout = params.socket_timeout;

        Ok(Sqlite {
            connection,
            file_path,
            socket_timeout,
        })
    }

    pub async fn attach_database(&mut self, db_name: &str) -> crate::Result<()> {
        let mut conn = self.connection.lock().await;

        let databases: HashSet<String> = sqlx::query("PRAGMA database_list")
            .try_map(|row: SqliteRow| {
                let name: String = row.try_get(1)?;
                Ok(name)
            })
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .collect();

        if !databases.contains(db_name) {
            sqlx::query("ATTACH DATABASE ? AS ?")
                .bind(self.file_path.as_str())
                .bind(db_name)
                .execute(&mut *conn)
                .await?;
        }

        sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *conn).await?;

        Ok(())
    }

    async fn timeout<T, F, E>(&self, f: F) -> crate::Result<T>
    where
        F: Future<Output = std::result::Result<T, E>>,
        E: Into<Error>,
    {
        match self.socket_timeout {
            Some(duration) => match timeout(duration, f).await {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(err)) => Err(err.into()),
                Err(to) => Err(to.into()),
            },
            None => match f.await {
                Ok(result) => Ok(result),
                Err(err) => Err(err.into()),
            },
        }
    }
}

impl TransactionCapable for Sqlite {}

#[async_trait]
impl Queryable for Sqlite {
    async fn query(&self, q: Query<'_>) -> crate::Result<ResultSet> {
        let (sql, params) = visitor::Sqlite::build(q)?;
        self.query_raw_new(&sql, params).await
    }

    async fn execute(&self, q: Query<'_>) -> crate::Result<u64> {
        let (sql, params) = visitor::Sqlite::build(q)?;
        self.execute_raw_new(&sql, params).await
    }

    async fn query_raw(&self, _: &str, _: &[Value<'_>]) -> crate::Result<ResultSet> {
        todo!()
    }

    async fn query_raw_new(&self, sql: &str, params: Vec<Value<'_>>) -> crate::Result<ResultSet> {
        metrics::query_new("sqlite.query_raw", sql, params, move |params| async move {
            let mut query = sqlx::query(sql);

            for param in params.into_iter() {
                query = query.bind_value(param)?;
            }

            let mut conn = self.connection.lock().await;
            let describe = self.timeout(conn.describe(sql)).await?;
            let columns = describe.columns.into_iter().map(|c| c.name).collect();

            let rows = query
                .try_map(|row| conversion::map_row(row))
                .fetch_all(&mut *conn)
                .await?;

            Ok(ResultSet::new(columns, rows))
        })
        .await
    }

    async fn execute_raw_new(&self, sql: &str, params: Vec<Value<'_>>) -> crate::Result<u64> {
        metrics::query_new("sqlite.execute_raw", sql, params, |params| async move {
            let mut query = sqlx::query(sql);

            for param in params.into_iter() {
                query = query.bind_value(param)?;
            }

            let mut conn = self.connection.lock().await;
            let changes = query.execute(&mut *conn).await?;

            Ok(changes)
        })
        .await
    }

    async fn execute_raw(&self, _: &str, _: &[Value<'_>]) -> crate::Result<u64> {
        todo!()
    }

    async fn raw_cmd(&self, cmd: &str) -> crate::Result<()> {
        metrics::query_new("sqlite.raw_cmd", cmd, Vec::new(), move |_| async move {
            let mut conn = self.connection.lock().await;
            sqlx::query(cmd).execute(&mut *conn).await?;
            Ok(())
        })
        .await
    }

    async fn version(&self) -> crate::Result<Option<String>> {
        let query = r#"SELECT sqlite_version() version;"#;
        let rows = self.query_raw_new(query, vec![]).await?;

        let version_string = rows
            .get(0)
            .and_then(|row| row.get("version").and_then(|version| version.to_string()));

        Ok(version_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ast::*,
        col,
        connector::{Queryable, TransactionCapable},
        error::{DatabaseConstraint, ErrorKind},
        single::Quaint,
        val, values,
    };

    #[test]
    fn sqlite_params_from_str_should_resolve_path_correctly_with_file_scheme() {
        let path = "file:dev.db";
        let params = SqliteParams::try_from(path).unwrap();
        assert_eq!(params.file_path, "dev.db");
    }

    #[test]
    fn sqlite_params_from_str_should_resolve_path_correctly_with_sqlite_scheme() {
        let path = "sqlite:dev.db";
        let params = SqliteParams::try_from(path).unwrap();
        assert_eq!(params.file_path, "dev.db");
    }

    #[test]
    fn sqlite_params_from_str_should_resolve_path_correctly_with_no_scheme() {
        let path = "dev.db";
        let params = SqliteParams::try_from(path).unwrap();
        assert_eq!(params.file_path, "dev.db");
    }

    #[tokio::test(threaded_scheduler)]
    async fn should_provide_a_database_connection() {
        let connection = Sqlite::new("db/test.db").await.unwrap();
        let res = connection
            .query_raw_new("SELECT * FROM sqlite_master", vec![])
            .await
            .unwrap();

        assert!(res.is_empty());
    }

    #[tokio::test(threaded_scheduler)]
    async fn should_provide_a_database_transaction() {
        let connection = Sqlite::new("db/test.db").await.unwrap();
        let tx = connection.start_transaction().await.unwrap();
        let res = tx.query_raw_new("SELECT * FROM sqlite_master", vec![]).await.unwrap();

        assert!(res.is_empty());
    }

    #[tokio::test(threaded_scheduler)]
    async fn test_aliased_value() {
        let conn = Sqlite::new("db/test.db").await.unwrap();
        let query = Select::default().value(val!(1).alias("test"));
        let rows = conn.select(query).await.unwrap();
        let row = rows.get(0).unwrap();

        assert_eq!(Value::integer(1), row["test"]);
    }

    #[tokio::test(threaded_scheduler)]
    async fn test_aliased_null() {
        let conn = Sqlite::new("db/test.db").await.unwrap();
        let query = Select::default().value(val!(Option::<i64>::None).alias("test"));
        let rows = conn.select(query).await.unwrap();
        let row = rows.get(0).unwrap();

        assert!(row["test"].is_null());
    }

    #[tokio::test(threaded_scheduler)]
    async fn tuples_in_selection() {
        let table = r#"
            CREATE TABLE tuples (id SERIAL PRIMARY KEY, age INTEGER NOT NULL, length REAL NOT NULL);
        "#;

        let connection = Quaint::new("file:db/test.db").await.unwrap();

        connection.raw_cmd("DROP TABLE IF EXISTS tuples").await.unwrap();
        connection.raw_cmd(table).await.unwrap();

        let insert = Insert::multi_into("tuples", vec!["age", "length"])
            .values(vec![val!(35), val!(20.0)])
            .values(vec![val!(40), val!(18.0)]);

        connection.insert(insert.into()).await.unwrap();

        // 1-tuple
        {
            let mut cols = Row::new();
            cols.push(Column::from("age"));

            let mut vals = Row::new();
            vals.push(35);

            let select = Select::from_table("tuples").so_that(cols.in_selection(vals));
            let rows = connection.select(select).await.unwrap();

            let row = rows.get(0).unwrap();
            assert_eq!(row["age"].as_i64(), Some(35));
            assert_eq!(row["length"].as_f64(), Some(20.0));
        }

        // 2-tuple
        {
            let cols = Row::from((col!("age"), col!("length")));
            let vals = values!((35, 20.0));

            let select = Select::from_table("tuples").so_that(cols.in_selection(vals));
            let rows = connection.select(select).await.unwrap();

            let row = rows.get(0).unwrap();
            assert_eq!(row["age"].as_i64(), Some(35));
            assert_eq!(row["length"].as_f64(), Some(20.0));
        }
    }

    #[allow(unused)]
    const TABLE_DEF: &str = r#"
    CREATE TABLE USER (
        ID INT PRIMARY KEY     NOT NULL,
        NAME           TEXT    NOT NULL,
        AGE            INT     NOT NULL,
        SALARY         REAL
    );
    "#;

    #[allow(unused)]
    const CREATE_USER: &str = r#"
    INSERT INTO USER (ID,NAME,AGE,SALARY)
    VALUES (1, 'Joe', 27, 20000.00 );
    "#;

    #[tokio::test(threaded_scheduler)]
    async fn should_map_columns_correctly() {
        let connection = Sqlite::new("file:db/test.db").await.unwrap();

        connection.raw_cmd(TABLE_DEF).await.unwrap();

        let changes = connection.execute_raw_new(CREATE_USER, vec![]).await.unwrap();
        assert_eq!(1, changes);

        let rows = connection.query_raw_new("SELECT * FROM USER", vec![]).await.unwrap();
        assert_eq!(rows.len(), 1);

        let row = rows.get(0).unwrap();
        assert_eq!(row["ID"].as_i64(), Some(1));
        assert_eq!(row["NAME"].as_str(), Some("Joe"));
        assert_eq!(row["AGE"].as_i64(), Some(27));
        assert_eq!(row["SALARY"].as_f64(), Some(20000.0));
    }

    #[tokio::test(threaded_scheduler)]
    async fn op_test_add_one_level() {
        let connection = Sqlite::new("file:db/test.db").await.unwrap();
        let q = Select::default().value(val!(2) + val!(1));

        let rows = connection.select(q).await.unwrap();
        let row = rows.get(0).unwrap();

        assert_eq!(row[0].as_i64(), Some(3));
    }

    #[tokio::test(threaded_scheduler)]
    async fn op_test_add_two_levels() {
        let connection = Sqlite::new("file:db/test.db").await.unwrap();
        let q = Select::default().value(val!(2) + val!(val!(3) + val!(2)));

        let rows = connection.select(q).await.unwrap();
        let row = rows.get(0).unwrap();

        assert_eq!(row[0].as_i64(), Some(7));
    }

    #[tokio::test(threaded_scheduler)]
    async fn op_test_sub_one_level() {
        let connection = Sqlite::new("file:db/test.db").await.unwrap();
        let q = Select::default().value(val!(2) - val!(1));

        let rows = connection.select(q).await.unwrap();
        let row = rows.get(0).unwrap();

        assert_eq!(row[0].as_i64(), Some(1));
    }

    #[tokio::test(threaded_scheduler)]
    async fn op_test_sub_three_items() {
        let connection = Sqlite::new("file:db/test.db").await.unwrap();
        let q = Select::default().value(val!(2) - val!(1) - val!(1));

        let rows = connection.select(q).await.unwrap();
        let row = rows.get(0).unwrap();

        assert_eq!(row[0].as_i64(), Some(0));
    }

    #[tokio::test(threaded_scheduler)]
    async fn op_test_sub_two_levels() {
        let connection = Sqlite::new("file:db/test.db").await.unwrap();
        let q = Select::default().value(val!(2) - val!(val!(3) + val!(1)));

        let rows = connection.select(q).await.unwrap();
        let row = rows.get(0).unwrap();

        assert_eq!(row[0].as_i64(), Some(-2));
    }

    #[tokio::test(threaded_scheduler)]
    async fn op_test_mul_one_level() {
        let connection = Sqlite::new("file:db/test.db").await.unwrap();
        let q = Select::default().value(val!(6) * val!(6));

        let rows = connection.select(q).await.unwrap();
        let row = rows.get(0).unwrap();

        assert_eq!(row[0].as_i64(), Some(36));
    }

    #[tokio::test(threaded_scheduler)]
    async fn op_test_mul_two_levels() {
        let connection = Sqlite::new("file:db/test.db").await.unwrap();
        let q = Select::default().value(val!(6) * (val!(6) - val!(1)));

        let rows = connection.select(q).await.unwrap();
        let row = rows.get(0).unwrap();

        assert_eq!(row[0].as_i64(), Some(30));
    }

    #[tokio::test(threaded_scheduler)]
    async fn op_multiple_operations() {
        let connection = Sqlite::new("file:db/test.db").await.unwrap();
        let q = Select::default().value(val!(4) - val!(2) * val!(2));

        let rows = connection.select(q).await.unwrap();
        let row = rows.get(0).unwrap();

        assert_eq!(row[0].as_i64(), Some(0));
    }

    #[tokio::test(threaded_scheduler)]
    async fn op_test_div_one_level() {
        let connection = Sqlite::new("file:db/test.db").await.unwrap();
        let q = Select::default().value(val!(6) / val!(3));

        let rows = connection.select(q).await.unwrap();
        let row = rows.get(0).unwrap();

        assert_eq!(row[0].as_i64(), Some(2));
    }

    #[tokio::test(threaded_scheduler)]
    async fn test_uniq_constraint_violation() {
        let conn = Sqlite::new("file:db/test.db").await.unwrap();

        let _ = conn.raw_cmd("DROP TABLE test_uniq_constraint_violation").await;

        conn.raw_cmd("CREATE TABLE test_uniq_constraint_violation (id1 int, id2 int)")
            .await
            .unwrap();
        conn.raw_cmd("CREATE UNIQUE INDEX musti ON test_uniq_constraint_violation (id1, id2)")
            .await
            .unwrap();

        conn.raw_cmd("INSERT INTO test_uniq_constraint_violation (id1, id2) VALUES (1, 2)")
            .await
            .unwrap();

        let res = conn
            .raw_cmd("INSERT INTO test_uniq_constraint_violation (id1, id2) VALUES (1, 2)")
            .await;

        let err = res.unwrap_err();

        match err.kind() {
            ErrorKind::UniqueConstraintViolation { constraint } => {
                assert_eq!(Some("2067"), err.original_code());
                assert_eq!(Some("UNIQUE constraint failed: test_uniq_constraint_violation.id1, test_uniq_constraint_violation.id2"), err.original_message());

                assert_eq!(
                    &DatabaseConstraint::Fields(vec![String::from("id1"), String::from("id2")]),
                    constraint,
                )
            }
            _ => panic!(err),
        }
    }

    #[tokio::test(threaded_scheduler)]
    async fn test_null_constraint_violation() {
        let conn = Sqlite::new("file:db/test.db").await.unwrap();

        let _ = conn.raw_cmd("DROP TABLE test_null_constraint_violation").await;

        conn.raw_cmd("CREATE TABLE test_null_constraint_violation (id1 int not null, id2 int not null)")
            .await
            .unwrap();

        let res = conn
            .query_raw_new("INSERT INTO test_null_constraint_violation DEFAULT VALUES", vec![])
            .await;

        let err = res.unwrap_err();

        match dbg!(err.kind()) {
            ErrorKind::NullConstraintViolation { constraint } => {
                assert_eq!(Some("1299"), err.original_code());
                assert_eq!(
                    Some("NOT NULL constraint failed: test_null_constraint_violation.id1"),
                    err.original_message()
                );
                assert_eq!(&DatabaseConstraint::Fields(vec![String::from("id1")]), constraint)
            }
            _ => panic!(err),
        }
    }

    #[tokio::test]
    async fn upper_fun() {
        let conn = Sqlite::try_from("file:db/test.db").unwrap();
        let select = Select::default().value(upper("foo").alias("val"));

        let res = conn.query(select.into()).await.unwrap();
        let row = res.get(0).unwrap();
        let val = row.get("val").unwrap().as_str();

        assert_eq!(Some("FOO"), val);
    }

    #[tokio::test]
    async fn lower_fun() {
        let conn = Sqlite::try_from("file:db/test.db").unwrap();
        let select = Select::default().value(lower("BAR").alias("val"));

        let res = conn.query(select.into()).await.unwrap();
        let row = res.get(0).unwrap();
        let val = row.get("val").unwrap().as_str();

        assert_eq!(Some("bar"), val);
    }
}
