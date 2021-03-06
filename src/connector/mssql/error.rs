use crate::error::{DatabaseConstraint, Error, ErrorKind};

impl From<tiberius::error::Error> for Error {
    fn from(e: tiberius::error::Error) -> Error {
        match e {
            tiberius::error::Error::Server(e) if e.code() == 18456 => {
                let user = e.message().split('\'').nth(1).unwrap().to_string();
                let mut builder = Error::builder(ErrorKind::AuthenticationFailed { user });

                builder.set_original_code(format!("{}", e.code()));
                builder.set_original_message(e.message().to_string());

                builder.build()
            }
            tiberius::error::Error::Server(e) if e.code() == 4060 => {
                let db_name = e.message().split('"').nth(1).unwrap().to_string();
                let mut builder = Error::builder(ErrorKind::DatabaseDoesNotExist { db_name });

                builder.set_original_code(format!("{}", e.code()));
                builder.set_original_message(e.message().to_string());

                builder.build()
            }
            tiberius::error::Error::Server(e) if e.code() == 515 => {
                let column = e.message().split('"').nth(1).unwrap().to_string();

                let mut builder = Error::builder(ErrorKind::NullConstraintViolation {
                    constraint: DatabaseConstraint::Fields(vec![column]),
                });

                builder.set_original_code(format!("{}", e.code()));
                builder.set_original_message(e.message().to_string());

                builder.build()
            }
            tiberius::error::Error::Server(e) if e.code() == 1801 => {
                let db_name = e.message().split('\'').nth(1).unwrap().to_string();

                let mut builder = Error::builder(ErrorKind::DatabaseAlreadyExists { db_name });

                builder.set_original_code(format!("{}", e.code()));
                builder.set_original_message(e.message().to_string());

                builder.build()
            }
            tiberius::error::Error::Server(e) if e.code() == 2627 => {
                let index = e
                    .message()
                    .split('.')
                    .nth(1)
                    .unwrap()
                    .split(' ')
                    .last()
                    .unwrap()
                    .split("'")
                    .nth(1)
                    .unwrap();

                let mut builder = Error::builder(ErrorKind::UniqueConstraintViolation {
                    constraint: DatabaseConstraint::Index(index.to_string()),
                });

                builder.set_original_code(format!("{}", e.code()));
                builder.set_original_message(e.message().to_string());

                builder.build()
            }
            tiberius::error::Error::Server(e) if e.code() == 547 => {
                let index = e.message().split(' ').nth(8).unwrap().split("\"").nth(1).unwrap();

                let mut builder = Error::builder(ErrorKind::ForeignKeyConstraintViolation {
                    constraint: DatabaseConstraint::Index(index.to_string()),
                });

                builder.set_original_code(format!("{}", e.code()));
                builder.set_original_message(e.message().to_string());

                builder.build()
            }
            tiberius::error::Error::Server(e) if e.code() == 2601 => {
                let index = e.message().split(' ').nth(9).unwrap().split("\"").nth(1).unwrap();

                let mut builder = Error::builder(ErrorKind::ForeignKeyConstraintViolation {
                    constraint: DatabaseConstraint::Index(index.to_string()),
                });

                builder.set_original_code(format!("{}", e.code()));
                builder.set_original_message(e.message().to_string());

                builder.build()
            }
            tiberius::error::Error::Server(e) if e.code() == 2714 => {
                let db_name = e.message().split('\'').nth(1).unwrap().to_string();
                let mut builder = Error::builder(ErrorKind::DatabaseAlreadyExists { db_name });

                builder.set_original_code(format!("{}", e.code()));
                builder.set_original_message(e.message().to_string());

                builder.build()
            }
            tiberius::error::Error::Server(e) if e.code() == 2628 => {
                let column_name = e.message().split('\'').nth(3).unwrap().to_string();

                let mut builder = Error::builder(ErrorKind::LengthMismatch {
                    column: Some(column_name.to_owned()),
                });

                builder.set_original_code(format!("{}", e.code()));
                builder.set_original_message(e.message().to_string());

                builder.build()
            }
            tiberius::error::Error::Server(e) => {
                let kind = ErrorKind::QueryError(e.clone().into());

                let mut builder = Error::builder(kind);
                builder.set_original_code(format!("{}", e.code()));
                builder.set_original_message(e.message().to_string());

                builder.build()
            }
            e => Error::builder(ErrorKind::QueryError(e.into())).build(),
        }
    }
}
