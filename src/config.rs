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
    
    // Check for empty repo list
    if config.repos.is_empty() {
        errors.push(ValidationError::EmptyRepoList);
    }
    
    // Track names for duplicate detection
    let mut seen_names = std::collections::HashSet::new();
    
    for (index, repo) in config.repos.iter().enumerate() {
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
    config
        .repos
        .iter()
        .map(|repo| repo.clone().with_defaults(&config.defaults))
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

    #[test]
    fn test_validate_empty_repos() {
        let config = Config {
            version: "1.0".to_string(),
            defaults: Defaults::default(),
            repos: vec![],
        };
        
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_duplicate_names() {
        let config = Config {
            version: "1.0".to_string(),
            defaults: Defaults::default(),
            repos: vec![
                RepoConfig {
                    name: "test".to_string(),
                    url: "https://github.com/test/test1.git".to_string(),
                    branch: None,
                    depth: None,
                    enabled: true,
                },
                RepoConfig {
                    name: "test".to_string(),
                    url: "https://github.com/test/test2.git".to_string(),
                    branch: None,
                    depth: None,
                    enabled: true,
                },
            ],
        };
        
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_validate_valid_config() {
        let config = Config {
            version: "1.0".to_string(),
            defaults: Defaults::default(),
            repos: vec![
                RepoConfig {
                    name: "repo1".to_string(),
                    url: "https://github.com/test/repo1.git".to_string(),
                    branch: None,
                    depth: None,
                    enabled: true,
                },
                RepoConfig {
                    name: "repo2".to_string(),
                    url: "git@github.com:test/repo2.git".to_string(),
                    branch: Some("develop".to_string()),
                    depth: Some(5),
                    enabled: true,
                },
            ],
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
            repos: vec![
                RepoConfig {
                    name: "repo1".to_string(),
                    url: "https://github.com/test/repo1.git".to_string(),
                    branch: None,
                    depth: None,
                    enabled: true,
                },
                RepoConfig {
                    name: "repo2".to_string(),
                    url: "https://github.com/test/repo2.git".to_string(),
                    branch: Some("main".to_string()),
                    depth: Some(1),
                    enabled: true,
                },
            ],
        };
        
        let repos = apply_defaults(&config);
        
        assert_eq!(repos[0].branch(), "develop");
        assert_eq!(repos[0].depth(), 10);
        assert_eq!(repos[1].branch(), "main");
        assert_eq!(repos[1].depth(), 1);
    }

    #[test]
    fn test_filter_enabled() {
        let repos = vec![
            RepoConfig {
                name: "enabled".to_string(),
                url: "https://github.com/test/enabled.git".to_string(),
                branch: None,
                depth: None,
                enabled: true,
            },
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
