use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use uuid::Uuid;

use crate::types::{ExecutionRecord, Language, LanguageStats, StatsResponse};

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(db_path: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .connect(&format!("sqlite:{}", db_path))
            .await?;

        Ok(Self { pool })
    }

    pub async fn init(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS executions (
                id TEXT PRIMARY KEY,
                language TEXT NOT NULL,
                success INTEGER NOT NULL,
                stdout TEXT NOT NULL,
                stderr TEXT NOT NULL,
                execution_time_ms INTEGER NOT NULL,
                error_message TEXT,
                code TEXT NOT NULL,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_executions_language ON executions(language);
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_executions_created_at ON executions(created_at DESC);
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn record_execution(
        &self,
        id: Uuid,
        language: Language,
        success: bool,
        stdout: &str,
        stderr: &str,
        execution_time_ms: u64,
        error_message: Option<&str>,
        code: &str,
    ) -> Result<()> {
        let now: DateTime<Utc> = Utc::now();

        sqlx::query(
            r#"
            INSERT INTO executions (
                id, language, success, stdout, stderr,
                execution_time_ms, error_message, code, created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id.to_string())
        .bind(language.as_str())
        .bind(success)
        .bind(stdout)
        .bind(stderr)
        .bind(execution_time_ms as i64)
        .bind(error_message)
        .bind(code)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_stats(&self) -> Result<StatsResponse> {
        let total: Option<i64> = sqlx::query_scalar("SELECT COUNT(*) FROM executions")
            .fetch_one(&self.pool)
            .await?;

        let successful: Option<i64> = sqlx::query_scalar(
            "SELECT COUNT(*) FROM executions WHERE success = 1",
        )
        .fetch_one(&self.pool)
        .await?;

        let avg_time: Option<f64> = sqlx::query_scalar(
            "SELECT AVG(execution_time_ms) FROM executions",
        )
        .fetch_one(&self.pool)
        .await?;

        let total_executions = total.unwrap_or(0) as u64;
        let successful_executions = successful.unwrap_or(0) as u64;
        let failed_executions = total_executions - successful_executions;
        let average_execution_time_ms = avg_time.unwrap_or(0.0);

        let by_language = self.get_language_stats().await?;
        let recent_executions = self.get_recent_executions(10).await?;

        Ok(StatsResponse {
            total_executions,
            successful_executions,
            failed_executions,
            average_execution_time_ms,
            by_language,
            recent_executions,
        })
    }

    async fn get_language_stats(&self) -> Result<Vec<LanguageStats>> {
        let rows: Vec<(String, i64, i64, f64)> = sqlx::query_as(
            r#"
            SELECT
                language,
                COUNT(*) as count,
                SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as success_count,
                AVG(execution_time_ms) as avg_time
            FROM executions
            GROUP BY language
            ORDER BY count DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut stats = Vec::new();
        for (lang_str, count, success_count, avg_time) in rows {
            if let Some(language) = Language::from_str(&lang_str) {
                stats.push(LanguageStats {
                    language,
                    count: count as u64,
                    success_count: success_count as u64,
                    average_time_ms: avg_time,
                });
            }
        }

        Ok(stats)
    }

    async fn get_recent_executions(&self, limit: u32) -> Result<Vec<ExecutionRecord>> {
        let rows: Vec<(
            String,
            String,
            bool,
            String,
            String,
            i64,
            Option<String>,
            String,
        )> = sqlx::query_as(
            r#"
            SELECT
                id, language, success, stdout, stderr,
                execution_time_ms, error_message, created_at
            FROM executions
            ORDER BY created_at DESC
            LIMIT ?
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut records = Vec::new();
        for (
            id_str,
            lang_str,
            success,
            stdout,
            stderr,
            execution_time_ms,
            error_message,
            created_at_str,
        ) in rows
        {
            if let (Ok(id), Some(language), Ok(created_at)) = (
                Uuid::parse_str(&id_str),
                Language::from_str(&lang_str),
                DateTime::parse_from_rfc3339(&created_at_str),
            ) {
                records.push(ExecutionRecord {
                    id,
                    language,
                    success,
                    stdout,
                    stderr,
                    execution_time_ms: execution_time_ms as u64,
                    error_message,
                    created_at: created_at.with_timezone(&Utc),
                });
            }
        }

        Ok(records)
    }
}
