use super::Handler;

/// Handler for Prisma CLI (`prisma generate`, `prisma migrate`, `prisma db`, `prisma studio`).
pub struct PrismaHandler;

impl Handler for PrismaHandler {
    fn filter(&self, output: &str, args: &[String]) -> String {
        let subcmd = args.get(1).map(|s| s.as_str()).unwrap_or("");
        match subcmd {
            "generate" => filter_generate(output),
            "migrate" => filter_migrate(output, args),
            "db" => filter_db(output, args),
            "studio" => filter_studio(output),
            "validate" => filter_validate(output),
            "format" => filter_format(output),
            _ => filter_generic(output),
        }
    }
}

fn filter_generate(output: &str) -> String {
    let mut generated: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.contains("error") || t.starts_with("Error") {
            errors.push(t.to_string());
            continue;
        }
        if t.contains("warn") || t.contains("deprecated") {
            if warnings.len() < 3 {
                warnings.push(t.to_string());
            }
            continue;
        }
        // Key lines: "✔ Generated Prisma Client (v5.0.0) to ./node_modules/... in 234ms"
        if t.starts_with("✔") || t.starts_with("✓") || t.contains("Generated") {
            generated.push(t.to_string());
            continue;
        }
        // Drop: "Environment variables loaded", "Prisma schema loaded from", progress dots
    }

    if !errors.is_empty() {
        return errors.join("\n");
    }

    let mut out = generated;
    out.extend(warnings);
    if out.is_empty() {
        "generate complete".to_string()
    } else {
        out.join("\n")
    }
}

fn filter_migrate(output: &str, args: &[String]) -> String {
    let migrate_subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("");
    let mut migrations: Vec<String> = Vec::new();
    let mut result_line: Option<String> = None;
    let mut errors: Vec<String> = Vec::new();

    for line in output.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.contains("error") || t.starts_with("Error") {
            errors.push(t.to_string());
            continue;
        }
        // Migration file names
        if t.contains("migration.sql") || (t.starts_with("└─") && t.contains("_")) {
            migrations.push(t.to_string());
            continue;
        }
        // Result: "Your database is now in sync with your schema."
        if t.contains("in sync") || t.contains("applied") || t.contains("migrations have been applied") {
            result_line = Some(t.to_string());
            continue;
        }
        // "The following migration(s) have been created and applied"
        if t.contains("migration(s)") || t.contains("migrations/") {
            migrations.push(t.to_string());
            continue;
        }
    }

    if !errors.is_empty() {
        return errors.join("\n");
    }

    let mut out: Vec<String> = Vec::new();
    if !migrations.is_empty() {
        out.push(format!(
            "[{} migration(s) {}]",
            migrations.len(),
            if migrate_subcmd == "deploy" { "deployed" } else { "applied" }
        ));
        for m in migrations.iter().take(5) {
            out.push(format!("  {}", m));
        }
        if migrations.len() > 5 {
            out.push(format!("  [+{} more]", migrations.len() - 5));
        }
    }
    if let Some(r) = result_line {
        out.push(r);
    }
    if out.is_empty() {
        "migrate complete".to_string()
    } else {
        out.join("\n")
    }
}

fn filter_db(output: &str, args: &[String]) -> String {
    let db_subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("");
    match db_subcmd {
        "push" => {
            // Keep: "Your database is now in sync with your schema."
            // Keep: error lines
            let mut out: Vec<String> = Vec::new();
            for line in output.lines() {
                let t = line.trim();
                if t.contains("in sync") || t.contains("error") || t.contains("Error") || t.starts_with("✔") || t.starts_with("✓") {
                    out.push(t.to_string());
                }
            }
            if out.is_empty() {
                "db push complete".to_string()
            } else {
                out.join("\n")
            }
        }
        "seed" => {
            // Keep last meaningful line
            let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
            lines.last().map(|l| l.trim().to_string()).unwrap_or_else(|| "seed complete".to_string())
        }
        _ => filter_generic(output),
    }
}

fn filter_studio(output: &str) -> String {
    // Keep the URL line
    for line in output.lines() {
        let t = line.trim();
        if t.contains("http://") || t.contains("https://") || t.contains("Studio is up") {
            return t.to_string();
        }
    }
    output.lines().last().unwrap_or("studio started").trim().to_string()
}

fn filter_validate(output: &str) -> String {
    for line in output.lines() {
        let t = line.trim();
        if t.contains("error") || t.contains("Error") {
            return t.to_string();
        }
    }
    "Schema is valid.".to_string()
}

fn filter_format(output: &str) -> String {
    let formatted: Vec<&str> = output
        .lines()
        .filter(|l| l.trim().ends_with(".prisma"))
        .collect();
    if formatted.is_empty() {
        return "Already formatted.".to_string();
    }
    format!("[{} file(s) formatted]", formatted.len())
}

fn filter_generic(output: &str) -> String {
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() <= 10 {
        return output.to_string();
    }
    // Keep errors + last 5 lines
    let mut out: Vec<String> = lines
        .iter()
        .filter(|l| {
            let lower = l.to_lowercase();
            lower.contains("error") || lower.contains("warn") || lower.contains("✔") || lower.contains("✓")
        })
        .map(|l| l.trim().to_string())
        .collect();
    let tail = &lines[lines.len().saturating_sub(5)..];
    out.extend(tail.iter().map(|l| l.trim().to_string()));
    out.dedup();
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn args(subcmd: &str) -> Vec<String> {
        vec!["prisma".to_string(), subcmd.to_string()]
    }

    fn args2(subcmd: &str, sub2: &str) -> Vec<String> {
        vec!["prisma".to_string(), subcmd.to_string(), sub2.to_string()]
    }

    #[test]
    fn generate_extracts_client_line() {
        let output = "\
Environment variables loaded from .env
Prisma schema loaded from prisma/schema.prisma
✔ Generated Prisma Client (v5.0.0) to ./node_modules/@prisma/client in 234ms
";
        let result = PrismaHandler.filter(output, &args("generate"));
        assert!(result.contains("Generated") || result.contains("✔"));
        assert!(!result.contains("Environment variables"));
    }

    #[test]
    fn generate_error_shown() {
        let output = "\
Environment variables loaded from .env
Prisma schema loaded from prisma/schema.prisma
error: Error validating model \"User\": The `id` field is missing.\n";
        let result = PrismaHandler.filter(output, &args("generate"));
        assert!(result.contains("error"));
    }

    #[test]
    fn migrate_extracts_migration_names() {
        let output = "\
Prisma schema loaded from prisma/schema.prisma
Datasource \"db\": PostgreSQL database at localhost:5432

The following migration(s) have been created and applied:
migrations/
  └─ 20231104_add_user_profile/
    └─ migration.sql

Your database is now in sync with your schema.
";
        let result = PrismaHandler.filter(output, &args("migrate"));
        assert!(result.contains("migration") || result.contains("sync"));
    }

    #[test]
    fn db_push_keeps_sync_message() {
        let output = "\
Prisma schema loaded from prisma/schema.prisma
✔ Your database is now in sync with your schema.\n";
        let result = PrismaHandler.filter(output, &args2("db", "push"));
        assert!(result.contains("sync") || result.contains("✔"));
    }

    #[test]
    fn validate_clean_schema() {
        let output = "The schema at prisma/schema.prisma is valid.\n";
        let result = PrismaHandler.filter(output, &args("validate"));
        assert!(!result.is_empty());
    }
}
