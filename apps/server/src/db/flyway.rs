//! Faithful re-implementation of the Flyway bits hawier must be
//! byte-compatible with: the `schema_version` history table, the line-based
//! CRC32 checksum, and filename → (version, description) parsing.
//!
//! TEMPORARY (dev stage): hawier keeps writing Flyway-shaped history rows so
//! a stock hawkBit 1.0.x can still boot against the same database without a
//! `flyway repair`. Verify once against a stock hawkbit-update-server boot
//! when registry access is available (see README "DB compatibility").

/// CRC32 (IEEE) — matches `java.util.zip.CRC32` used by Flyway.
fn crc32(data_lines: impl Iterator<Item = impl AsRef<[u8]>>) -> u32 {
    const fn table() -> [u32; 256] {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                k += 1;
            }
            table[i] = c;
            i += 1;
        }
        table
    }
    const TABLE: [u32; 256] = table();

    let mut crc: u32 = 0xFFFF_FFFF;
    for line in data_lines {
        for &byte in line.as_ref() {
            crc = TABLE[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
    }
    !crc
}

/// Flyway's migration checksum: CRC32 over the file's lines with line
/// terminators excluded (Java `BufferedReader.readLine` semantics), BOM
/// stripped from the first line. Stored as a signed INTEGER.
pub fn checksum(sql: &str) -> i32 {
    let sql = sql.strip_prefix('\u{FEFF}').unwrap_or(sql);
    crc32(sql.lines().map(|l| l.as_bytes())) as i32
}

/// One migration hawier knows how to apply, with the metadata Flyway would
/// record for it.
pub struct Migration {
    /// `schema_version.version`, e.g. "1.20.0".
    pub version: &'static str,
    /// `schema_version.description` — parsed from the filename exactly like
    /// Flyway's `MigrationInfoHelper` (underscores → spaces, no trim).
    pub description: &'static str,
    /// `schema_version.type`: `SQL_BASELINE` for `B`-prefixed files, `SQL`.
    pub kind: &'static str,
    /// `schema_version.script` — resource path relative to `db/migration`.
    pub script: &'static str,
    pub sql: &'static str,
}

/// The exact hawkBit 1.0.x PostgreSQL migration chain (copied verbatim from
/// hawkbit-repository-jpa-flyway; suffix filter is `POSTGRESQL.sql`).
pub const HAWKBIT_MIGRATIONS: &[Migration] = &[
    Migration {
        version: "1.20.0",
        // "B1_20_0__1.0.0_baseline__POSTGRESQL.sql" minus prefix "B" and
        // suffix "POSTGRESQL.sql" → "1_20_0__1.0.0_baseline__"; description
        // part "1.0.0_baseline__" → underscores to spaces.
        description: "1.0.0 baseline  ",
        kind: "SQL_BASELINE",
        script: "POSTGRESQL/B1_20_0__1.0.0_baseline__POSTGRESQL.sql",
        sql: include_str!("../../migrations/hawkbit/B1_20_0__1.0.0_baseline__POSTGRESQL.sql"),
    },
    Migration {
        version: "1.20.1",
        description: "spring boot 4  ",
        kind: "SQL",
        script: "POSTGRESQL/V1_20_1__spring_boot_4__POSTGRESQL.sql",
        sql: include_str!("../../migrations/hawkbit/V1_20_1__spring_boot_4__POSTGRESQL.sql"),
    },
    Migration {
        version: "1.20.2",
        description: "action rollout indexes  ",
        kind: "SQL",
        script: "POSTGRESQL/V1_20_2__action_rollout_indexes__POSTGRESQL.sql",
        sql: include_str!(
            "../../migrations/hawkbit/V1_20_2__action_rollout_indexes__POSTGRESQL.sql"
        ),
    },
];

/// Flyway's history-table DDL for PostgreSQL (table name `schema_version`
/// per hawkbit-jpa-flyway-defaults.properties).
pub const HISTORY_TABLE_DDL: &str = r#"
CREATE TABLE "schema_version" (
    "installed_rank" INT NOT NULL,
    "version" VARCHAR(50),
    "description" VARCHAR(200) NOT NULL,
    "type" VARCHAR(20) NOT NULL,
    "script" VARCHAR(1000) NOT NULL,
    "checksum" INTEGER,
    "installed_by" VARCHAR(100) NOT NULL,
    "installed_on" TIMESTAMP NOT NULL DEFAULT now(),
    "execution_time" INTEGER NOT NULL,
    "success" BOOLEAN NOT NULL,
    CONSTRAINT "schema_version_pk" PRIMARY KEY ("installed_rank")
);
CREATE INDEX "schema_version_s_idx" ON "schema_version" ("success");
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_java_reference() {
        // java.util.zip.CRC32 of "hello" == 0x3610A686
        assert_eq!(crc32(["hello"].iter().map(|s| s.as_bytes())), 0x3610_A686);
    }

    #[test]
    fn checksum_ignores_line_terminators() {
        assert_eq!(checksum("a\nb\n"), checksum("a\r\nb"));
        assert_eq!(checksum("\u{FEFF}a\nb"), checksum("a\nb"));
    }

    #[test]
    fn migrations_embedded_and_ordered() {
        assert_eq!(HAWKBIT_MIGRATIONS.len(), 3);
        assert!(HAWKBIT_MIGRATIONS[0].sql.contains("sp_target"));
        assert!(HAWKBIT_MIGRATIONS[1].sql.contains("SP_LOCK"));
        assert!(HAWKBIT_MIGRATIONS[2].sql.contains("sp_action"));
    }
}
