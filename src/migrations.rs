// Copyright 2017-2019 Parity Technologies (UK) Ltd.
// This file is part of substrate-archive.

// substrate-archive is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// substrate-archive is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with substrate-archive.  If not, see <http://www.gnu.org/licenses/>.

use crate::error::Error as ArchiveError;
use refinery::config::{Config, ConfigDbType};
use std::env;

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("src/migrations");
}

/// Run all the migrations
/// Returns the database URL
///
/// # Panics
/// Panics if a required environment variable is not found
/// or if the environment variable contains invalid unicode
pub fn migrate(conf: MigrationConfig) -> Result<String, ArchiveError> {
    let parsed = parse(conf);
    let mut conn = Config::new(ConfigDbType::Postgres)
        .set_db_host(parsed.host.as_str())
        .set_db_port(parsed.port.as_str())
        .set_db_name(parsed.name.as_str());

    if let Some(u) = &parsed.user {
        conn = conn.set_db_user(u.as_str());
    }
    if let Some(p) = &parsed.pass {
        conn = conn.set_db_pass(p.as_str())
    }

    log::info!("Running migrations for database {}", parsed.name.as_str());

    embedded::migrations::runner().run(&mut conn)?;
    Ok(parsed.build_url())
}

#[derive(Debug, Clone)]
pub struct MigrationConfig {
    pub host: Option<String>,
    pub port: Option<String>,
    pub user: Option<String>,
    pub pass: Option<String>,
    pub name: Option<String>,
}

impl MigrationConfig {
    pub fn url(&self) -> String {
        let parsed = parse(self.clone());
        parsed.build_url()
    }
}

/// Internal struct
/// passed to build a Database URL
struct MigrateConfigParsed {
    host: String,
    port: String,
    user: Option<String>,
    pass: Option<String>,
    name: String,
}

impl MigrateConfigParsed {
    /// build a database url
    fn build_url(&self) -> String {
        let mut url: String = "postgres".to_string() + "://";

        if let Some(user) = &self.user {
            url = url + &user;
        }
        if let Some(pass) = &self.pass {
            url = url + ":" + &pass;
        }

        if self.user.is_some() {
            url = url + "@" + &self.host;
        } else {
            url = url + &self.host;
        }
        url = url + ":" + &self.port;
        url = url + "/" + &self.name;
        url
    }
}

fn parse(conf: MigrationConfig) -> MigrateConfigParsed {
    let host: String = conf
        .host
        .unwrap_or_else(|| process_var("DB_HOST").unwrap_or_else(|| "localhost".to_string()));

    let port: String = conf
        .port
        .unwrap_or_else(|| process_var("DB_PORT").unwrap_or_else(|| "5432".to_string()));

    let user = if conf.user.is_some() {
        conf.user
    } else {
        process_var("DB_USER")
    };

    let pass = if conf.pass.is_some() {
        conf.pass
    } else {
        process_var("DB_PASS")
    };

    let name: String = conf
        .name
        .unwrap_or_else(|| process_var("DB_NAME").expect("database name must be defined"));

    MigrateConfigParsed {
        host,
        port,
        user,
        pass,
        name,
    }
}

/// process an environment variable
/// if config does not contain the variable
/// we try to get the variable from the environment
/// If the variable is not required according to PostgreSQL
/// we return None.
///
/// # Panics
/// panics if the environment variable is found but
/// contains invalid Unicode data
fn process_var(name: &str) -> Option<String> {
    match env::var(name) {
        Ok(v) => Some(v),
        Err(e) => match e {
            env::VarError::NotPresent => {
                log::warn!(
                    "Environment Variable {} is not present, constructing URL with default {} ",
                    name,
                    name
                );
                None
            }
            env::VarError::NotUnicode(data) => {
                log::error!(
                    "Environment Variable {} found, but contains invalid unicode data: {:?}",
                    name,
                    data
                );
                panic!("Environment contains invalid unicode data");
            }
        },
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn should_create_correct_db_url() {
        let conf = MigrateConfigParsed {
            host: "localhost".to_string(),
            port: "5432".to_string(),
            user: Some("archive".to_string()),
            pass: Some("default".to_string()),
            name: "archive".to_string(),
        };
        let url = conf.build_url(&conf);
        assert_eq!(url, "postgres://archive:default@localhost:5432/archive");
    }
}
