//! Configuration loading and validation
//!
//! This module handles loading and validating the repos.yaml configuration file.

use std::path::Path;
use anyhow::{Context, Result, bail};
use crate::models::{Config, RepoConfig};

/// Load configuration from a YAML file
///
/// # Arguments
/// * `path` - Path to the repos.yaml configuration file
///
/// # Returns
/// * `Result<Config>` - Parsed configuration or error
pub fn load_config<P: AsRef<Path>>(path: P) -> Result<Config> {
    let path = path.as_ref();

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: Config = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    Ok(config)
}

/// Repositories the scanner should clone and scan: `repos_active` followed by
/// `repos_github_only`. `repos_deprecated` is intentionally excluded.
pub fn scannable_repos(config: &Config) -> Vec<RepoConfig> {
    config
        .repos_active
        .iter()
        .chain(config.repos_github_only.iter())
        .cloned()
        .collect()
}

/// Validation error types
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Empty repository list")]
    EmptyRepoList,
    
    #[error("Invalid URL for repository '{name}': {url}")]
    InvalidUrl { name: String, url: String },
    
    #[error("Duplicate repository name: {name}")]
    DuplicateName { name: String },
    
    #[error("Empty repository name at index {index}")]
    EmptyName { index: usize },
    
    #[error("Empty URL for repository '{name}'")]
    EmptyUrl { name: String },
}

/// Validate the configuration
///
/// Checks for:
/// - Non-empty repository list
/// - Valid URL formats (https:// or git@)
/// - Unique repository names
/// - Non-empty names and URLs
///
/// # Returns
/// * `Ok(())` if valid
/// * `Err` with list of validation errors
pub fn validate_config(config: &Config) -> Result<()> {
    let mut errors: Vec<ValidationError> = Vec::new();

    let repos = scannable_repos(config);

    // Check for empty repo list
    if repos.is_empty() {
        errors.push(ValidationError::EmptyRepoList);
    }

    // Track names for duplicate detection
    let mut seen_names = std::collections::HashSet::new();

    for (index, repo) in repos.iter().enumerate() {
        // Check for empty name
        if repo.name.trim().is_empty() {
            errors.push(ValidationError::EmptyName { index });
            continue;
        }
        
        // Check for duplicate names
        if !seen_names.insert(&repo.name) {
            errors.push(ValidationError::DuplicateName {
                name: repo.name.clone(),
            });
        }
        
        // Check for empty URL
        if repo.url.trim().is_empty() {
            errors.push(ValidationError::EmptyUrl {
                name: repo.name.clone(),
            });
            continue;
        }
        
        // Validate URL format
        if !is_valid_git_url(&repo.url) {
            errors.push(ValidationError::InvalidUrl {
                name: repo.name.clone(),
                url: repo.url.clone(),
            });
        }
    }
    
    if !errors.is_empty() {
        let error_messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        bail!("Configuration validation failed:\n  - {}", error_messages.join("\n  - "));
    }
    
    Ok(())
}

/// Check if a URL is a valid Git URL
fn is_valid_git_url(url: &str) -> bool {
    url.starts_with("https://") || 
    url.starts_with("http://") || 
    url.starts_with("git@") ||
    url.starts_with("ssh://")
}

/// Apply default values to all repository configurations
///
/// # Arguments
/// * `config` - The configuration to process
///
/// # Returns
/// * Vector of RepoConfig with defaults applied
pub fn apply_defaults(config: &Config) -> Vec<RepoConfig> {
    scannable_repos(config)
        .into_iter()
        .map(|repo| repo.with_defaults(&config.defaults))
        .collect()
}

/// Filter enabled repositories
///
/// # Arguments
/// * `repos` - List of repository configurations
///
/// # Returns
/// * Vector of enabled RepoConfig
pub fn filter_enabled(repos: Vec<RepoConfig>) -> Vec<RepoConfig> {
    repos.into_iter().filter(|r| r.enabled).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Defaults;

    #[test]
    fn test_is_valid_git_url() {
        assert!(is_valid_git_url("https://github.com/NVIDIA/test.git"));
        assert!(is_valid_git_url("http://github.com/NVIDIA/test.git"));
        assert!(is_valid_git_url("git@github.com:NVIDIA/test.git"));
        assert!(is_valid_git_url("ssh://git@github.com/NVIDIA/test.git"));
        
        assert!(!is_valid_git_url("ftp://example.com/test.git"));
        assert!(!is_valid_git_url("not-a-url"));
        assert!(!is_valid_git_url(""));
    }

    fn repo(name: &str, url: &str) -> RepoConfig {
        RepoConfig {
            name: name.to_string(),
            url: url.to_string(),
            branch: None,
            depth: None,
            enabled: true,
        }
    }

    #[test]
    fn test_validate_empty_repos() {
        let config = Config {
            version: "1.0".to_string(),
            defaults: Defaults::default(),
            repos_active: vec![],
            repos_github_only: vec![],
            repos_deprecated: vec![],
        };

        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_scannable_repos_combines_active_and_github_only() {
        let config = Config {
            version: "1.0".to_string(),
            defaults: Defaults::default(),
            repos_active: vec![repo("active", "https://github.com/test/active.git")],
            repos_github_only: vec![repo("gh", "https://github.com/test/gh.git")],
            // Deprecated names must never be scanned.
            repos_deprecated: vec!["old/deprecated".to_string()],
        };

        let repos = scannable_repos(&config);
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "active");
        assert_eq!(repos[1].name, "gh");
    }

    #[test]
    fn test_validate_duplicate_names() {
        // A name duplicated across the two scanned sections is a duplicate.
        let config = Config {
            version: "1.0".to_string(),
            defaults: Defaults::default(),
            repos_active: vec![repo("test", "https://github.com/test/test1.git")],
            repos_github_only: vec![repo("test", "https://github.com/test/test2.git")],
            repos_deprecated: vec![],
        };

        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_valid_config() {
        let config = Config {
            version: "1.0".to_string(),
            defaults: Defaults::default(),
            repos_active: vec![repo("repo1", "https://github.com/test/repo1.git")],
            repos_github_only: vec![RepoConfig {
                name: "repo2".to_string(),
                url: "git@github.com:test/repo2.git".to_string(),
                branch: Some("develop".to_string()),
                depth: Some(5),
                enabled: true,
            }],
            repos_deprecated: vec![],
        };

        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_apply_defaults() {
        let config = Config {
            version: "1.0".to_string(),
            defaults: Defaults {
                branch: "develop".to_string(),
                depth: 10,
            },
            repos_active: vec![repo("repo1", "https://github.com/test/repo1.git")],
            repos_github_only: vec![RepoConfig {
                name: "repo2".to_string(),
                url: "https://github.com/test/repo2.git".to_string(),
                branch: Some("main".to_string()),
                depth: Some(1),
                enabled: true,
            }],
            repos_deprecated: vec![],
        };

        let repos = apply_defaults(&config);

        // active first, then github_only
        assert_eq!(repos[0].branch(), "develop");
        assert_eq!(repos[0].depth(), 10);
        assert!(repos[0].enabled);
        assert_eq!(repos[1].branch(), "main");
        assert_eq!(repos[1].depth(), 1);
    }

    #[test]
    fn test_filter_enabled() {
        let repos = vec![
            repo("enabled", "https://github.com/test/enabled.git"),
            RepoConfig {
                name: "disabled".to_string(),
                url: "https://github.com/test/disabled.git".to_string(),
                branch: None,
                depth: None,
                enabled: false,
            },
        ];

        let filtered = filter_enabled(repos);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "enabled");
    }
}
