use anyhow::Result;
use std::path::PathBuf;

/// Build the file-relationship graph index for a repository.
pub fn run(repo_path: Option<String>) -> Result<()> {
    let repo_root = if let Some(path) = repo_path {
        PathBuf::from(path)
    } else {
        std::env::current_dir()?
    };

    // Get index parent directory: ~/.local/share/panda/indexes/<repo-hash>/
    let repo_hash = compute_repo_hash(&repo_root)?;
    let index_parent = get_index_parent(&repo_hash)?;

    println!("Building index for: {}", repo_root.display());
    println!("Index location: {}", index_parent.display());

    // Run the indexing
    panda_core::focus::run_index(&repo_root, &index_parent)?;

    println!("✓ Index built successfully");
    Ok(())
}

fn compute_repo_hash(repo_root: &std::path::Path) -> Result<String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let path_str = repo_root.to_string_lossy();
    let mut hasher = DefaultHasher::new();
    path_str.hash(&mut hasher);
    let hash = hasher.finish();
    Ok(format!("{:x}", hash))
}

fn get_index_parent(repo_hash: &str) -> Result<std::path::PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
    let parent = home.join(".local/share/panda/indexes").join(repo_hash);
    Ok(parent)
}
