use crate::error::{Error, ErrorKind};
use std::{borrow::Cow, fmt};
use url::Url;

#[cfg(feature = "mssql")]
use crate::connector::MssqlUrl;
#[cfg(feature = "mysql")]
use crate::connector::MysqlUrl;
#[cfg(feature = "postgresql")]
use crate::connector::PostgresUrl;
#[cfg(feature = "sqlite")]
use crate::connector::SqliteParams;
#[cfg(feature = "sqlite")]
use std::convert::TryFrom;

/// General information about a SQL connection.
#[derive(Debug, Clone)]
pub enum ConnectionInfo {
    /// A PostgreSQL connection URL.
    #[cfg(feature = "postgresql")]
    Postgres(PostgresUrl),
    /// A MySQL connection URL.
    #[cfg(feature = "mysql")]
    Mysql(MysqlUrl),
    /// A SQL Server connection URL.
    #[cfg(feature = "mssql")]
    Mssql(MssqlUrl),
    /// A SQLite connection URL.
    #[cfg(feature = "sqlite")]
    Sqlite {
        /// The filesystem path of the SQLite database.
        file_path: String,
        /// The name the database is bound to (with `ATTACH DATABASE`), if available.
        db_name: String,
    },
}

impl ConnectionInfo {
    /// Parse `ConnectionInfo` out from an SQL connection string.
    ///
    /// Will fail if URI is invalid or the scheme points to an unsupported
    /// database.
    pub fn from_url(url_str: &str) -> crate::Result<Self> {
        let url_result: Result<Url, _> = url_str.parse();

        // Non-URL database strings are interpreted as SQLite file paths.
        match url_str {
            #[cfg(feature = "sqlite")]
            s if s.starts_with("file") || s.starts_with("sqlite") => {
                if url_result.is_err() {
                    let params = SqliteParams::try_from(s)?;

                    return Ok(ConnectionInfo::Sqlite {
                        file_path: params.file_path,
                        db_name: params.db_name.clone(),
                    });
                }
            }
            #[cfg(feature = "mssql")]
            s if s.starts_with("jdbc:sqlserver") || s.starts_with("sqlserver") => {
                return Ok(ConnectionInfo::Mssql(MssqlUrl::new(url_str)?));
            }
            _ => (),
        }

        let url = url_result?;

        let sql_family = SqlFamily::from_scheme(url.scheme()).ok_or_else(|| {
            let kind =
                ErrorKind::DatabaseUrlIsInvalid(format!("{} is not a supported database URL scheme.", url.scheme()));

            Error::builder(kind).build()
        })?;

        match sql_family {
            #[cfg(feature = "mysql")]
            SqlFamily::Mysql => Ok(ConnectionInfo::Mysql(MysqlUrl::new(url)?)),
            #[cfg(feature = "sqlite")]
            SqlFamily::Sqlite => {
                let params = SqliteParams::try_from(url_str)?;

                Ok(ConnectionInfo::Sqlite {
                    file_path: params.file_path,
                    db_name: params.db_name,
                })
            }
            #[cfg(feature = "postgresql")]
            SqlFamily::Postgres => Ok(ConnectionInfo::Postgres(PostgresUrl::new(url)?)),
            _ => unreachable!(),
        }
    }

    /// The provided database name. This will be `None` on SQLite.
    pub fn dbname(&self) -> Option<&str> {
        match self {
            #[cfg(feature = "postgresql")]
            ConnectionInfo::Postgres(url) => Some(url.dbname()),
            #[cfg(feature = "mysql")]
            ConnectionInfo::Mysql(url) => Some(url.dbname()),
            #[cfg(feature = "mssql")]
            ConnectionInfo::Mssql(url) => Some(url.dbname()),
            #[cfg(feature = "sqlite")]
            ConnectionInfo::Sqlite { .. } => None,
        }
    }

    /// This is what item names are prefixed with in queries.
    ///
    /// - In SQLite, this is the schema name that the database file was attached as.
    /// - In Postgres, it is the selected schema inside the current database.
    /// - In MySQL, it is the database name.
    pub fn schema_name(&self) -> &str {
        match self {
            #[cfg(feature = "postgresql")]
            ConnectionInfo::Postgres(url) => url.schema(),
            #[cfg(feature = "mysql")]
            ConnectionInfo::Mysql(url) => url.dbname(),
            #[cfg(feature = "mssql")]
            ConnectionInfo::Mssql(url) => url.dbname(),
            #[cfg(feature = "sqlite")]
            ConnectionInfo::Sqlite { db_name, .. } => db_name,
        }
    }

    /// The provided database host. This will be `"localhost"` on SQLite.
    pub fn host(&self) -> &str {
        match self {
            #[cfg(feature = "postgresql")]
            ConnectionInfo::Postgres(url) => url.host(),
            #[cfg(feature = "mysql")]
            ConnectionInfo::Mysql(url) => url.host(),
            #[cfg(feature = "mssql")]
            ConnectionInfo::Mssql(url) => url.host(),
            #[cfg(feature = "sqlite")]
            ConnectionInfo::Sqlite { .. } => "localhost",
        }
    }

    /// The provided database user name. This will be `None` on SQLite.
    pub fn username(&self) -> Option<Cow<str>> {
        match self {
            #[cfg(feature = "postgresql")]
            ConnectionInfo::Postgres(url) => Some(url.username()),
            #[cfg(feature = "mysql")]
            ConnectionInfo::Mysql(url) => Some(url.username()),
            #[cfg(feature = "mssql")]
            ConnectionInfo::Mssql(url) => url.username().map(Cow::from),
            #[cfg(feature = "sqlite")]
            ConnectionInfo::Sqlite { .. } => None,
        }
    }

    /// The database file for SQLite, otherwise `None`.
    pub fn file_path(&self) -> Option<&str> {
        match self {
            #[cfg(feature = "postgresql")]
            ConnectionInfo::Postgres(_) => None,
            #[cfg(feature = "mysql")]
            ConnectionInfo::Mysql(_) => None,
            #[cfg(feature = "mssql")]
            ConnectionInfo::Mssql(_) => None,
            #[cfg(feature = "sqlite")]
            ConnectionInfo::Sqlite { file_path, .. } => Some(file_path),
        }
    }

    /// The family of databases connected.
    pub fn sql_family(&self) -> SqlFamily {
        match self {
            #[cfg(feature = "postgresql")]
            ConnectionInfo::Postgres(_) => SqlFamily::Postgres,
            #[cfg(feature = "mysql")]
            ConnectionInfo::Mysql(_) => SqlFamily::Mysql,
            #[cfg(feature = "mssql")]
            ConnectionInfo::Mssql(_) => SqlFamily::Mssql,
            #[cfg(feature = "sqlite")]
            ConnectionInfo::Sqlite { .. } => SqlFamily::Sqlite,
        }
    }

    /// The provided database port, if applicable.
    pub fn port(&self) -> Option<u16> {
        match self {
            #[cfg(feature = "postgresql")]
            ConnectionInfo::Postgres(url) => Some(url.port()),
            #[cfg(feature = "mysql")]
            ConnectionInfo::Mysql(url) => Some(url.port()),
            #[cfg(feature = "mssql")]
            ConnectionInfo::Mssql(url) => Some(url.port()),
            #[cfg(feature = "sqlite")]
            ConnectionInfo::Sqlite { .. } => None,
        }
    }

    /// A string describing the database location, meant for error messages. It will be the host
    /// and port on MySQL/Postgres, and the file path on SQLite.
    pub fn database_location(&self) -> String {
        match self {
            #[cfg(feature = "postgresql")]
            ConnectionInfo::Postgres(url) => format!("{}:{}", url.host(), url.port()),
            #[cfg(feature = "mysql")]
            ConnectionInfo::Mysql(url) => format!("{}:{}", url.host(), url.port()),
            #[cfg(feature = "mssql")]
            ConnectionInfo::Mssql(url) => format!("{}:{}", url.host(), url.port()),
            #[cfg(feature = "sqlite")]
            ConnectionInfo::Sqlite { file_path, .. } => file_path.clone(),
        }
    }
}

/// One of the supported SQL variants.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum SqlFamily {
    #[cfg(feature = "postgresql")]
    Postgres,
    #[cfg(feature = "mysql")]
    Mysql,
    #[cfg(feature = "sqlite")]
    Sqlite,
    #[cfg(feature = "mssql")]
    Mssql,
}

impl SqlFamily {
    /// Get a string representation of the family.
    pub fn as_str(self) -> &'static str {
        match self {
            #[cfg(feature = "postgresql")]
            SqlFamily::Postgres => "postgresql",
            #[cfg(feature = "mysql")]
            SqlFamily::Mysql => "mysql",
            #[cfg(feature = "sqlite")]
            SqlFamily::Sqlite => "sqlite",
            #[cfg(feature = "mssql")]
            SqlFamily::Mssql => "mssql",
        }
    }

    /// Convert url scheme to an SqlFamily.
    pub fn from_scheme(url_scheme: &str) -> Option<Self> {
        match url_scheme {
            #[cfg(feature = "sqlite")]
            "sqlite" | "file" => Some(SqlFamily::Sqlite),
            #[cfg(feature = "postgresql")]
            "postgres" | "postgresql" => Some(SqlFamily::Postgres),
            #[cfg(feature = "mysql")]
            "mysql" => Some(SqlFamily::Mysql),
            _ => None,
        }
    }

    /// Check if a family exists for the given scheme.
    pub fn scheme_is_supported(url_scheme: &str) -> bool {
        Self::from_scheme(url_scheme).is_some()
    }

    #[cfg(feature = "postgresql")]
    pub fn is_postgres(&self) -> bool {
        matches!(self, SqlFamily::Postgres)
    }

    #[cfg(feature = "mysql")]
    pub fn is_mysql(&self) -> bool {
        matches!(self, SqlFamily::Mysql)
    }

    #[cfg(feature = "sqlite")]
    pub fn is_sqlite(&self) -> bool {
        matches!(self, SqlFamily::Sqlite)
    }

    #[cfg(feature = "mssql")]
    pub fn is_mssql(&self) -> bool {
        matches!(self, SqlFamily::Mssql)
    }
}

impl fmt::Display for SqlFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "sqlite")]
    use super::*;

    #[test]
    #[cfg(feature = "sqlite")]
    fn sqlite_connection_info_from_str_interprets_relative_path_correctly() {
        let conn_info = ConnectionInfo::from_url("file:dev.db").unwrap();

        match conn_info {
            ConnectionInfo::Sqlite { file_path, db_name: _ } => assert_eq!(file_path, "dev.db"),
            _ => panic!("wrong"),
        }
    }
}
