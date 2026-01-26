//! Git operations for cloning and managing repositories
//!
//! This module handles cloning repositories and managing temporary directories.

use std::path::{Path, PathBuf};
use std::process::Command;
use anyhow::{Context, Result, bail};
use log::{info, warn, debug};
use rayon::prelude::*;

use crate::models::RepoConfig;

/// Inject GitHub token into HTTPS URL for private repo access
///
/// Converts: https://github.com/org/repo.git
/// To:       https://<token>@github.com/org/repo.git
fn inject_github_token(url: &str, token: &str) -> String {
    if url.starts_with("https://github.com/") {
        url.replace("https://github.com/", &format!("https://{}@github.com/", token))
    } else if url.starts_with("https://") {
        // For other HTTPS URLs, insert token after https://
        url.replace("https://", &format!("https://{}@", token))
    } else {
        // For SSH or other URLs, return as-is
        url.to_string()
    }
}

/// Result of a clone operation
#[derive(Debug)]
pub struct CloneResult {
    /// The repository configuration
    pub repo: RepoConfig,
    /// Path to the cloned repository (if successful)
    pub path: Option<PathBuf>,
    /// Error message (if failed)
    pub error: Option<String>,
}

impl CloneResult {
    /// Check if the clone was successful
    pub fn is_success(&self) -> bool {
        self.path.is_some()
    }
}

/// Clone a single repository
///
/// # Arguments
/// * `repo` - Repository configuration
/// * `workdir` - Working directory to clone into
/// * `github_token` - Optional GitHub token for private repos
///
/// # Returns
/// * `Result<PathBuf>` - Path to the cloned repository
pub fn clone_repo(repo: &RepoConfig, workdir: &Path, github_token: Option<&str>) -> Result<PathBuf> {
    // Create a safe directory name from the repo name
    let dir_name = repo.name.replace('/', "_").replace('\\', "_");
    let target_dir = workdir.join(&dir_name);
    
    // Reuse existing directory if present
    if target_dir.exists() {
        debug!("Reusing existing directory: {}", target_dir.display());
        if let Err(e) = update_existing_repo(repo, &target_dir) {
            warn!("Failed to update existing repo {}: {}", repo.name, e);
            // Fall back to using the existing checkout to avoid blocking scans
            return Ok(target_dir);
        }
        return Ok(target_dir);
    }
    
    info!("Cloning {} into {}", repo.name, target_dir.display());
    
    // Build clone URL (inject token for private repos if provided)
    let clone_url = if let Some(token) = github_token {
        inject_github_token(&repo.url, token)
    } else {
        repo.url.clone()
    };
    
    // Build git clone command
    let mut cmd = Command::new("git");
    cmd.arg("clone")
        .arg("--depth")
        .arg(repo.depth().to_string())
        .arg("--branch")
        .arg(repo.branch())
        .arg("--single-branch")
        .arg(&clone_url)
        .arg(&target_dir);
    
    // Log without exposing token
    debug!("Running: git clone --depth {} --branch {} --single-branch {} {}",
           repo.depth(), repo.branch(), repo.url, target_dir.display());
    
    // Execute the command
    let output = cmd
        .output()
        .with_context(|| format!("Failed to execute git clone for {}", repo.name))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Git clone failed for {}: {}", repo.name, stderr.trim());
    }
    
    info!("Successfully cloned {}", repo.name);
    Ok(target_dir)
}

/// Update an existing repository checkout
fn update_existing_repo(repo: &RepoConfig, target_dir: &Path) -> Result<()> {
    let branch = repo.branch();
    let depth = repo.depth();

    // Fetch latest changes (shallow fetch if depth provided)
    let mut fetch_cmd = Command::new("git");
    fetch_cmd
        .arg("-C")
        .arg(target_dir)
        .arg("fetch")
        .arg("origin")
        .arg(branch);
    if depth > 0 {
        fetch_cmd.arg("--depth").arg(depth.to_string());
    }
    let fetch_output = fetch_cmd
        .output()
        .with_context(|| format!("Failed to fetch {}", repo.name))?;
    if !fetch_output.status.success() {
        let stderr = String::from_utf8_lossy(&fetch_output.stderr);
        warn!("Git fetch failed for {}: {}", repo.name, stderr.trim());
    }

    // Ensure we are on the intended branch
    let checkout_output = Command::new("git")
        .arg("-C")
        .arg(target_dir)
        .arg("checkout")
        .arg(branch)
        .output()
        .with_context(|| format!("Failed to checkout {} {}", repo.name, branch))?;
    if !checkout_output.status.success() {
        let stderr = String::from_utf8_lossy(&checkout_output.stderr);
        warn!("Git checkout failed for {}: {}", repo.name, stderr.trim());
    }

    // Pull fast-forward only
    let pull_output = Command::new("git")
        .arg("-C")
        .arg(target_dir)
        .arg("pull")
        .arg("--ff-only")
        .arg("origin")
        .arg(branch)
        .output()
        .with_context(|| format!("Failed to pull {}", repo.name))?;
    if !pull_output.status.success() {
        let stderr = String::from_utf8_lossy(&pull_output.stderr);
        warn!("Git pull failed for {}: {}", repo.name, stderr.trim());
    }

    Ok(())
}

/// Clone all repositories in parallel
///
/// # Arguments
/// * `repos` - List of repository configurations
/// * `workdir` - Working directory to clone into
/// * `github_token` - Optional GitHub token for private repos
///
/// # Returns
/// * Vector of CloneResult for each repository
pub fn clone_all_repos(repos: &[RepoConfig], workdir: &Path, github_token: Option<&str>) -> Vec<CloneResult> {
    // Ensure workdir exists
    if let Err(e) = std::fs::create_dir_all(workdir) {
        warn!("Failed to create workdir {}: {}", workdir.display(), e);
    }
    
    repos
        .par_iter()
        .map(|repo| {
            match clone_repo(repo, workdir, github_token) {
                Ok(path) => CloneResult {
                    repo: repo.clone(),
                    path: Some(path),
                    error: None,
                },
                Err(e) => {
                    warn!("Failed to clone {}: {}", repo.name, e);
                    CloneResult {
                        repo: repo.clone(),
                        path: None,
                        error: Some(e.to_string()),
                    }
                }
            }
        })
        .collect()
}

/// Clean up cloned repositories
///
/// # Arguments
/// * `workdir` - Working directory containing cloned repositories
pub fn cleanup_repos(workdir: &Path) -> Result<()> {
    if workdir.exists() {
        info!("Cleaning up workdir: {}", workdir.display());
        std::fs::remove_dir_all(workdir)
            .with_context(|| format!("Failed to remove workdir: {}", workdir.display()))?;
    }
    Ok(())
}

/// Get statistics about clone results
pub fn clone_stats(results: &[CloneResult]) -> (usize, usize) {
    let success = results.iter().filter(|r| r.is_success()).count();
    let failed = results.len() - success;
    (success, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_clone_result_is_success() {
        let success = CloneResult {
            repo: RepoConfig {
                name: "test".to_string(),
                url: "https://github.com/test/test.git".to_string(),
                branch: None,
                depth: None,
                enabled: true,
            },
            path: Some(PathBuf::from("/tmp/test")),
            error: None,
        };
        assert!(success.is_success());

        let failure = CloneResult {
            repo: RepoConfig {
                name: "test".to_string(),
                url: "https://github.com/test/test.git".to_string(),
                branch: None,
                depth: None,
                enabled: true,
            },
            path: None,
            error: Some("Clone failed".to_string()),
        };
        assert!(!failure.is_success());
    }

    #[test]
    fn test_clone_stats() {
        let results = vec![
            CloneResult {
                repo: RepoConfig {
                    name: "repo1".to_string(),
                    url: "https://github.com/test/repo1.git".to_string(),
                    branch: None,
                    depth: None,
                    enabled: true,
                },
                path: Some(PathBuf::from("/tmp/repo1")),
                error: None,
            },
            CloneResult {
                repo: RepoConfig {
                    name: "repo2".to_string(),
                    url: "https://github.com/test/repo2.git".to_string(),
                    branch: None,
                    depth: None,
                    enabled: true,
                },
                path: None,
                error: Some("Failed".to_string()),
            },
        ];

        let (success, failed) = clone_stats(&results);
        assert_eq!(success, 1);
        assert_eq!(failed, 1);
    }

    // Integration test - requires network access
    #[test]
    #[ignore]
    fn test_clone_real_repo() {
        let temp_dir = TempDir::new().unwrap();
        let repo = RepoConfig {
            name: "test/hello-world".to_string(),
            url: "https://github.com/octocat/Hello-World.git".to_string(),
            branch: Some("master".to_string()),
            depth: Some(1),
            enabled: true,
        };

        let result = clone_repo(&repo, temp_dir.path(), None);
        assert!(result.is_ok());
        
        let path = result.unwrap();
        assert!(path.exists());
        assert!(path.join(".git").exists());
    }

    #[test]
    fn test_inject_github_token() {
        let url = "https://github.com/org/repo.git";
        let result = inject_github_token(url, "my-token");
        assert_eq!(result, "https://my-token@github.com/org/repo.git");
    }

    #[test]
    fn test_inject_github_token_other_host() {
        let url = "https://gitlab.com/org/repo.git";
        let result = inject_github_token(url, "my-token");
        assert_eq!(result, "https://my-token@gitlab.com/org/repo.git");
    }

    #[test]
    fn test_inject_github_token_ssh() {
        let url = "git@github.com:org/repo.git";
        let result = inject_github_token(url, "my-token");
        assert_eq!(result, "git@github.com:org/repo.git"); // unchanged
    }
}
