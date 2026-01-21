//! Data models for NIM Usage Scanner
//!
//! This module defines all data structures used throughout the scanner,
//! including configuration, scan results, and API responses.

use serde::{Deserialize, Serialize};

// ============================================================================
// Source Type Classification
// ============================================================================

/// Represents the source type of a NIM reference
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    /// Regular source code (not in .github/workflows/)
    SourceCode,
    /// GitHub Actions workflow files (.github/workflows/*.yml)
    ActionsWorkflow,
}

// ============================================================================
// Configuration Structures
// ============================================================================

/// Top-level configuration structure parsed from repos.yaml
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Configuration file version (reserved for future compatibility checks)
    #[allow(dead_code)]
    pub version: String,
    /// Default values for repository settings
    #[serde(default)]
    pub defaults: Defaults,
    /// List of repositories to scan
    pub repos: Vec<RepoConfig>,
}

/// Default configuration values
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Defaults {
    /// Default branch to clone
    #[serde(default = "default_branch")]
    pub branch: String,
    /// Default clone depth
    #[serde(default = "default_depth")]
    pub depth: u32,
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_depth() -> u32 {
    1
}

/// Configuration for a single repository
#[derive(Debug, Clone, Deserialize)]
pub struct RepoConfig {
    /// Repository identifier name (used in reports)
    pub name: String,
    /// Git clone URL
    pub url: String,
    /// Branch to clone (overrides defaults)
    pub branch: Option<String>,
    /// Clone depth (overrides defaults)
    pub depth: Option<u32>,
    /// Whether this repo is enabled for scanning
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl RepoConfig {
    /// Apply default values from Defaults struct
    pub fn with_defaults(mut self, defaults: &Defaults) -> Self {
        if self.branch.is_none() {
            self.branch = Some(defaults.branch.clone());
        }
        if self.depth.is_none() {
            self.depth = Some(defaults.depth);
        }
        self
    }

    /// Get the branch to clone
    pub fn branch(&self) -> &str {
        self.branch.as_deref().unwrap_or("main")
    }

    /// Get the clone depth
    pub fn depth(&self) -> u32 {
        self.depth.unwrap_or(1)
    }
}

// ============================================================================
// Scan Result Structures
// ============================================================================

/// A detected Local NIM reference (Docker image from nvcr.io/nim/*)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalNimMatch {
    /// Repository name where the match was found
    pub repository: String,
    /// Full image URL (e.g., nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2)
    pub image_url: String,
    /// Image tag/version (e.g., 1.10.0 or latest)
    pub tag: String,
    /// Resolved tag if original was 'latest' (from NGC API)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_tag: Option<String>,
    /// File path relative to repository root
    pub file_path: String,
    /// Line number where the match was found (1-indexed)
    pub line_number: usize,
    /// The actual line content that matched
    pub match_context: String,
}

/// A detected Hosted NIM reference (API endpoint to *.api.nvidia.com)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostedNimMatch {
    /// Repository name where the match was found
    pub repository: String,
    /// API endpoint URL (e.g., https://ai.api.nvidia.com/v1)
    pub endpoint_url: Option<String>,
    /// Model name (e.g., nvidia/llama-3.1-nemotron-70b-instruct)
    pub model_name: Option<String>,
    /// File path relative to repository root
    pub file_path: String,
    /// Line number where the match was found (1-indexed)
    pub line_number: usize,
    /// The actual line content that matched
    pub match_context: String,
    /// NVCF Function ID (populated by NGC API)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    /// Function status (e.g., ACTIVE, INACTIVE)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Underlying container image
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_image: Option<String>,
}

/// Collection of NIM findings for a specific source type
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NimFindings {
    /// Local NIM matches (Docker images)
    pub local_nim: Vec<LocalNimMatch>,
    /// Hosted NIM matches (API endpoints)
    pub hosted_nim: Vec<HostedNimMatch>,
}

impl NimFindings {
    /// Create a new empty NimFindings
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if there are any findings
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.local_nim.is_empty() && self.hosted_nim.is_empty()
    }

    /// Get the total count of findings
    #[allow(dead_code)]
    pub fn total_count(&self) -> usize {
        self.local_nim.len() + self.hosted_nim.len()
    }
}

// ============================================================================
// Report Structures
// ============================================================================

/// Complete scan report with results categorized by source type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    /// Timestamp when the scan was performed
    pub scan_time: String,
    /// Total number of repositories scanned
    pub total_repos: usize,
    /// NIM findings from regular source code
    pub source_code: NimFindings,
    /// NIM findings from GitHub Actions workflows
    pub actions_workflow: NimFindings,
    /// Aggregated view: NIMs grouped with all their locations
    pub aggregated: AggregatedFindings,
    /// Summary statistics
    pub summary: Summary,
}

/// Summary statistics for the scan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    /// Total number of Local NIM references found
    pub total_local_nim: usize,
    /// Total number of Hosted NIM references found
    pub total_hosted_nim: usize,
    /// Number of repositories containing at least one NIM reference
    pub repos_with_nim: usize,
    /// Statistics for source code findings
    pub source_code: CategorySummary,
    /// Statistics for workflow findings
    pub actions_workflow: CategorySummary,
}

/// Summary for a single category (source_code or actions_workflow)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CategorySummary {
    /// Number of Local NIM references
    pub local_nim: usize,
    /// Number of Hosted NIM references
    pub hosted_nim: usize,
}

// ============================================================================
// Aggregated View Structures (grouped by NIM)
// ============================================================================

/// Location where a NIM reference was found
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NimLocation {
    /// Source type: source_code or actions_workflow
    pub source_type: String,
    /// Repository name
    pub repository: String,
    /// File path within the repository
    pub file_path: String,
    /// Line number in the file
    pub line_number: usize,
    /// The matched line content
    pub match_context: String,
}

/// Aggregated Local NIM entry with all locations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedLocalNim {
    /// Full image URL (e.g., nvcr.io/nim/nvidia/llama3)
    pub image_url: String,
    /// Image tag/version
    pub tag: String,
    /// Resolved tag if original was 'latest' (from NGC API)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_tag: Option<String>,
    /// All locations where this NIM was found
    pub locations: Vec<NimLocation>,
}

/// Aggregated Hosted NIM entry with all locations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedHostedNim {
    /// API endpoint URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    /// Model name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// Function ID from NGC API
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    /// Function status from NGC API
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Container image from NGC API
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_image: Option<String>,
    /// All locations where this NIM was found
    pub locations: Vec<NimLocation>,
}

/// Aggregated view of all NIM findings grouped by NIM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedFindings {
    /// All unique Local NIMs with their locations
    pub local_nim: Vec<AggregatedLocalNim>,
    /// All unique Hosted NIMs with their locations
    pub hosted_nim: Vec<AggregatedHostedNim>,
}

// ============================================================================
// NGC API Response Structures
// ============================================================================

/// Response from NGC Container Registry API for repository info
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct NgcRepoResponse {
    /// Repository name
    pub name: Option<String>,
    /// Latest tag for the repository
    #[serde(rename = "latestTag")]
    pub latest_tag: Option<String>,
    /// Latest version ID
    #[serde(rename = "latestVersionId")]
    pub latest_version_id: Option<String>,
    /// Repository description
    pub description: Option<String>,
}

/// Response from NVCF Functions List API
#[derive(Debug, Clone, Deserialize)]
pub struct NgcFunctionListResponse {
    /// List of functions
    pub functions: Vec<NgcFunctionSummary>,
}

/// Summary of a single function from the list
#[derive(Debug, Clone, Deserialize)]
pub struct NgcFunctionSummary {
    /// Function ID
    pub id: String,
    /// Function name
    pub name: String,
    /// Function status
    pub status: Option<String>,
}

/// Response from NVCF Function Details API
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct NgcFunctionDetailsResponse {
    /// Function details
    pub function: NgcFunctionDetails,
}

/// Detailed information about a function
#[derive(Debug, Clone, Deserialize)]
pub struct NgcFunctionDetails {
    /// Function ID
    pub id: String,
    /// Function name
    pub name: String,
    /// Function status
    pub status: Option<String>,
    /// Container image used by the function
    #[serde(rename = "containerImage")]
    pub container_image: Option<String>,
}

// ============================================================================
// Helper Implementations
// ============================================================================

impl ScanReport {
    /// Create a new ScanReport with the given data
    pub fn new(
        total_repos: usize,
        source_code: NimFindings,
        actions_workflow: NimFindings,
    ) -> Self {
        let summary = Summary::calculate(&source_code, &actions_workflow);
        let aggregated = AggregatedFindings::from_findings(&source_code, &actions_workflow);
        
        Self {
            scan_time: chrono::Utc::now().to_rfc3339(),
            total_repos,
            source_code,
            actions_workflow,
            aggregated,
            summary,
        }
    }
}

impl AggregatedFindings {
    /// Create aggregated view from source_code and actions_workflow findings
    pub fn from_findings(source_code: &NimFindings, actions_workflow: &NimFindings) -> Self {
        use std::collections::HashMap;
        
        // Aggregate Local NIMs by (image_url, tag)
        let mut local_map: HashMap<(String, String), AggregatedLocalNim> = HashMap::new();
        
        for m in &source_code.local_nim {
            let key = (m.image_url.clone(), m.tag.clone());
            let entry = local_map.entry(key).or_insert_with(|| AggregatedLocalNim {
                image_url: m.image_url.clone(),
                tag: m.tag.clone(),
                resolved_tag: m.resolved_tag.clone(),
                locations: Vec::new(),
            });
            entry.locations.push(NimLocation {
                source_type: "source_code".to_string(),
                repository: m.repository.clone(),
                file_path: m.file_path.clone(),
                line_number: m.line_number,
                match_context: m.match_context.clone(),
            });
        }
        
        for m in &actions_workflow.local_nim {
            let key = (m.image_url.clone(), m.tag.clone());
            let entry = local_map.entry(key).or_insert_with(|| AggregatedLocalNim {
                image_url: m.image_url.clone(),
                tag: m.tag.clone(),
                resolved_tag: m.resolved_tag.clone(),
                locations: Vec::new(),
            });
            entry.locations.push(NimLocation {
                source_type: "actions_workflow".to_string(),
                repository: m.repository.clone(),
                file_path: m.file_path.clone(),
                line_number: m.line_number,
                match_context: m.match_context.clone(),
            });
        }
        
        // Aggregate Hosted NIMs by model_name (or endpoint_url if no model)
        let mut hosted_map: HashMap<String, AggregatedHostedNim> = HashMap::new();
        
        for m in &source_code.hosted_nim {
            let key = m.model_name.clone()
                .or_else(|| m.endpoint_url.clone())
                .unwrap_or_else(|| format!("unknown-{}", m.line_number));
            
            let entry = hosted_map.entry(key).or_insert_with(|| AggregatedHostedNim {
                endpoint_url: m.endpoint_url.clone(),
                model_name: m.model_name.clone(),
                function_id: m.function_id.clone(),
                status: m.status.clone(),
                container_image: m.container_image.clone(),
                locations: Vec::new(),
            });
            entry.locations.push(NimLocation {
                source_type: "source_code".to_string(),
                repository: m.repository.clone(),
                file_path: m.file_path.clone(),
                line_number: m.line_number,
                match_context: m.match_context.clone(),
            });
        }
        
        for m in &actions_workflow.hosted_nim {
            let key = m.model_name.clone()
                .or_else(|| m.endpoint_url.clone())
                .unwrap_or_else(|| format!("unknown-{}", m.line_number));
            
            let entry = hosted_map.entry(key).or_insert_with(|| AggregatedHostedNim {
                endpoint_url: m.endpoint_url.clone(),
                model_name: m.model_name.clone(),
                function_id: m.function_id.clone(),
                status: m.status.clone(),
                container_image: m.container_image.clone(),
                locations: Vec::new(),
            });
            entry.locations.push(NimLocation {
                source_type: "actions_workflow".to_string(),
                repository: m.repository.clone(),
                file_path: m.file_path.clone(),
                line_number: m.line_number,
                match_context: m.match_context.clone(),
            });
        }
        
        Self {
            local_nim: local_map.into_values().collect(),
            hosted_nim: hosted_map.into_values().collect(),
        }
    }
}

impl Summary {
    /// Calculate summary statistics from findings
    pub fn calculate(source_code: &NimFindings, actions_workflow: &NimFindings) -> Self {
        use std::collections::HashSet;
        
        // Collect all unique repositories
        let mut repos: HashSet<&str> = HashSet::new();
        
        for m in &source_code.local_nim {
            repos.insert(&m.repository);
        }
        for m in &source_code.hosted_nim {
            repos.insert(&m.repository);
        }
        for m in &actions_workflow.local_nim {
            repos.insert(&m.repository);
        }
        for m in &actions_workflow.hosted_nim {
            repos.insert(&m.repository);
        }
        
        Self {
            total_local_nim: source_code.local_nim.len() + actions_workflow.local_nim.len(),
            total_hosted_nim: source_code.hosted_nim.len() + actions_workflow.hosted_nim.len(),
            repos_with_nim: repos.len(),
            source_code: CategorySummary {
                local_nim: source_code.local_nim.len(),
                hosted_nim: source_code.hosted_nim.len(),
            },
            actions_workflow: CategorySummary {
                local_nim: actions_workflow.local_nim.len(),
                hosted_nim: actions_workflow.hosted_nim.len(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_type_serialization() {
        assert_eq!(
            serde_json::to_string(&SourceType::SourceCode).unwrap(),
            "\"source_code\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::ActionsWorkflow).unwrap(),
            "\"actions_workflow\""
        );
    }

    #[test]
    fn test_repo_config_defaults() {
        let defaults = Defaults {
            branch: "develop".to_string(),
            depth: 5,
        };
        
        let config = RepoConfig {
            name: "test".to_string(),
            url: "https://github.com/test/test.git".to_string(),
            branch: None,
            depth: None,
            enabled: true,
        };
        
        let config = config.with_defaults(&defaults);
        assert_eq!(config.branch(), "develop");
        assert_eq!(config.depth(), 5);
    }

    #[test]
    fn test_nim_findings_empty() {
        let findings = NimFindings::new();
        assert!(findings.is_empty());
        assert_eq!(findings.total_count(), 0);
    }

    #[test]
    fn test_summary_calculation() {
        let source_code = NimFindings {
            local_nim: vec![
                LocalNimMatch {
                    repository: "repo1".to_string(),
                    image_url: "nvcr.io/nim/nvidia/test".to_string(),
                    tag: "1.0.0".to_string(),
                    resolved_tag: None,
                    file_path: "Dockerfile".to_string(),
                    line_number: 1,
                    match_context: "FROM nvcr.io/nim/nvidia/test:1.0.0".to_string(),
                },
            ],
            hosted_nim: vec![],
        };
        
        let actions_workflow = NimFindings {
            local_nim: vec![],
            hosted_nim: vec![
                HostedNimMatch {
                    repository: "repo2".to_string(),
                    endpoint_url: Some("https://ai.api.nvidia.com/v1".to_string()),
                    model_name: Some("nvidia/test".to_string()),
                    file_path: ".github/workflows/test.yml".to_string(),
                    line_number: 10,
                    match_context: "model: nvidia/test".to_string(),
                    function_id: None,
                    status: None,
                    container_image: None,
                },
            ],
        };
        
        let summary = Summary::calculate(&source_code, &actions_workflow);
        assert_eq!(summary.total_local_nim, 1);
        assert_eq!(summary.total_hosted_nim, 1);
        assert_eq!(summary.repos_with_nim, 2);
        assert_eq!(summary.source_code.local_nim, 1);
        assert_eq!(summary.actions_workflow.hosted_nim, 1);
    }
}
