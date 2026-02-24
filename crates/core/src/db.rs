use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug)]
pub struct Db {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct Account {
    pub id: i64,
    pub alias: String,
    pub address: String,
    pub read_only: bool,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        // Minimal schema. Project A stores more metadata; we can expand later.
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;

            CREATE TABLE IF NOT EXISTS accounts (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              alias TEXT NOT NULL,
              address TEXT NOT NULL UNIQUE,
              read_only INTEGER NOT NULL DEFAULT 1,
              is_default INTEGER NOT NULL DEFAULT 0,
              created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_accounts_default ON accounts(is_default);
            "#,
        )?;
        Ok(())
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, alias, address, read_only, is_default, created_at FROM accounts ORDER BY is_default DESC, id ASC",
        )?;

        let rows = stmt.query_map([], |row| {
            let created_at: String = row.get(5)?;
            let created_at = created_at
                .parse::<DateTime<Utc>>()
                .unwrap_or_else(|_| Utc::now());
            Ok(Account {
                id: row.get(0)?,
                alias: row.get(1)?,
                address: row.get(2)?,
                read_only: row.get::<_, i64>(3)? != 0,
                is_default: row.get::<_, i64>(4)? != 0,
                created_at,
            })
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn add_account(
        &mut self,
        alias: &str,
        address: &str,
        read_only: bool,
        make_default: bool,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        let tx = self.conn.transaction()?;
        if make_default {
            tx.execute("UPDATE accounts SET is_default = 0", [])?;
        }
        let ins = tx.execute(
            "INSERT INTO accounts(alias, address, read_only, is_default, created_at) VALUES(?, ?, ?, ?, ?)",
            params![
                alias,
                address,
                if read_only { 1 } else { 0 },
                if make_default { 1 } else { 0 },
                now
            ],
        );
        match ins {
            Ok(_) => {
                tx.commit()?;
                Ok(())
            }
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.extended_code == 2067 || err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                // 2067: SQLITE_CONSTRAINT_UNIQUE
                Err(anyhow!(
                    "address already exists in accounts.db: {address}\n\
                     Run: hl account ls (and optionally: hl account set-default)"
                ))
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_default_by_id(&mut self, id: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE accounts SET is_default = 0", [])?;
        let n = tx.execute("UPDATE accounts SET is_default = 1 WHERE id = ?", params![id])?;
        if n == 0 {
            return Err(anyhow!("account id not found: {id}"));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_by_id(&mut self, id: i64) -> Result<()> {
        let n = self.conn.execute("DELETE FROM accounts WHERE id = ?", params![id])?;
        if n == 0 {
            return Err(anyhow!("account id not found: {id}"));
        }
        Ok(())
    }

    pub fn default_account(&self) -> Result<Option<Account>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, alias, address, read_only, is_default, created_at FROM accounts WHERE is_default = 1 LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let created_at: String = row.get(5)?;
            let created_at = created_at
                .parse::<DateTime<Utc>>()
                .unwrap_or_else(|_| Utc::now());
            Ok(Some(Account {
                id: row.get(0)?,
                alias: row.get(1)?,
                address: row.get(2)?,
                read_only: row.get::<_, i64>(3)? != 0,
                is_default: row.get::<_, i64>(4)? != 0,
                created_at,
            }))
        } else {
            Ok(None)
        }
    }
}
