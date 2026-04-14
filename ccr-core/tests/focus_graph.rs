//! Unit tests for focus graph — SQLite schema, connection management, file/cochange upserts.

use tempfile::tempdir;
use std::path::Path;

#[test]
fn test_graph_create_and_reopen() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("graph.sqlite");

    // Create fresh database
    let conn = panda_core::focus::open_readwrite(&db_path).unwrap();
    drop(conn);

    // Reopen should work without error
    let conn2 = panda_core::focus::open_readwrite(&db_path).unwrap();
    drop(conn2);
}

#[test]
fn test_graph_is_valid_missing_file() {
    let missing = Path::new("/nonexistent/graph.sqlite");
    assert!(!panda_core::focus::graph_is_valid(missing));
}

#[test]
fn test_graph_is_valid_fresh_db() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("graph.sqlite");

    panda_core::focus::open_readwrite(&db_path).unwrap();
    assert!(panda_core::focus::graph_is_valid(&db_path));
}

#[test]
fn test_graph_insert_and_retrieve_file() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("graph.sqlite");
    let conn = panda_core::focus::open_readwrite(&db_path).unwrap();

    // Insert a file
    conn.execute(
        "INSERT INTO files (path, role, role_confidence, commit_count, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params!["src/auth.rs", "entry_point", 0.91_f64, 5_i64, 1000_i64],
    )
    .unwrap();

    // Retrieve it
    let (role, confidence): (String, f64) = conn
        .query_row(
            "SELECT role, role_confidence FROM files WHERE path = 'src/auth.rs'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();

    assert_eq!(role, "entry_point");
    assert!((confidence - 0.91).abs() < 1e-6);
}

#[test]
fn test_graph_insert_and_retrieve_cochange() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("graph.sqlite");
    let conn = panda_core::focus::open_readwrite(&db_path).unwrap();

    // Insert a cochange pair
    conn.execute(
        "INSERT INTO cochanges (file_a, file_b, change_count, last_seen)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["src/auth.rs", "src/session.rs", 12_i64, 2000_i64],
    )
    .unwrap();

    // Retrieve it
    let count: i64 = conn
        .query_row(
            "SELECT change_count FROM cochanges WHERE file_a = 'src/auth.rs'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    assert_eq!(count, 12);
}

#[test]
fn test_graph_cochange_lexicographic_check() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("graph.sqlite");
    let conn = panda_core::focus::open_readwrite(&db_path).unwrap();

    // file_a must be < file_b lexicographically (enforced by CHECK constraint)
    let result = conn.execute(
        "INSERT INTO cochanges (file_a, file_b, change_count, last_seen)
         VALUES ('src/z.rs', 'src/a.rs', 1, 1000)",
        [],
    );

    // Should fail due to CHECK constraint
    assert!(result.is_err());
}

#[test]
fn test_graph_cochange_replace_on_duplicate() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("graph.sqlite");
    let conn = panda_core::focus::open_readwrite(&db_path).unwrap();

    // Insert initial pair
    conn.execute(
        "INSERT INTO cochanges (file_a, file_b, change_count, last_seen)
         VALUES ('src/a.rs', 'src/b.rs', 5, 1000)",
        [],
    )
    .unwrap();

    // Update count (in reality done via ON CONFLICT DO UPDATE)
    conn.execute(
        "UPDATE cochanges SET change_count = 10, last_seen = 2000
         WHERE file_a = 'src/a.rs' AND file_b = 'src/b.rs'",
        [],
    )
    .unwrap();

    let (count, last_seen): (i64, i64) = conn
        .query_row(
            "SELECT change_count, last_seen FROM cochanges
             WHERE file_a = 'src/a.rs' AND file_b = 'src/b.rs'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();

    assert_eq!(count, 10);
    assert_eq!(last_seen, 2000);
}

#[test]
fn test_graph_readonly_mode() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("graph.sqlite");

    // Create database
    panda_core::focus::open_readwrite(&db_path).unwrap();

    // Open read-only
    let conn = panda_core::focus::open_readonly(&db_path).unwrap();

    // Should be able to read
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
        .unwrap();

    assert_eq!(count, 0);
}

#[test]
fn test_graph_readonly_fails_on_missing() {
    let missing = std::path::PathBuf::from("/nonexistent/missing.sqlite");
    let result = panda_core::focus::open_readonly(&missing);
    assert!(result.is_err());
}

#[test]
fn test_graph_multiple_files_and_cochanges() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("graph.sqlite");
    let conn = panda_core::focus::open_readwrite(&db_path).unwrap();

    // Insert multiple files
    let files = vec![
        ("src/main.rs", "entry_point"),
        ("src/db.rs", "persistence"),
        ("src/models.rs", "state_manager"),
        ("src/utils.rs", "general"),
    ];

    for (path, role) in &files {
        conn.execute(
            "INSERT INTO files (path, role, role_confidence, commit_count, updated_at)
             VALUES (?1, ?2, 0.8, 1, 1000)",
            rusqlite::params![path, role],
        )
        .unwrap();
    }

    // Verify count
    let count: usize = conn
        .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 4);

    // Insert cochanges
    let pairs = vec![
        ("src/db.rs", "src/main.rs", 10),
        ("src/db.rs", "src/models.rs", 8),
        ("src/main.rs", "src/utils.rs", 3),
    ];

    for (a, b, count) in &pairs {
        let (a_sort, b_sort) = if a < b { (a, b) } else { (b, a) };
        conn.execute(
            "INSERT INTO cochanges (file_a, file_b, change_count, last_seen)
             VALUES (?1, ?2, ?3, 1000)",
            rusqlite::params![a_sort, b_sort, count],
        )
        .unwrap();
    }

    let cochange_count: usize = conn
        .query_row("SELECT COUNT(*) FROM cochanges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cochange_count, 3);
}
