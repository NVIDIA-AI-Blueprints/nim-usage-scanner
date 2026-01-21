//! Static code scanner for NIM references
//!
//! This module implements the core scanning logic to detect Local NIM (Docker images)
//! and Hosted NIM (API endpoints) references in source code.

use std::path::Path;
use regex::Regex;
use once_cell::sync::Lazy;
use log::{debug, warn};
use ignore::WalkBuilder;
use rayon::prelude::*;

use crate::models::{LocalNimMatch, HostedNimMatch, NimFindings, SourceType};

// ============================================================================
// Regex Patterns
// ============================================================================

/// Local NIM patterns - matches nvcr.io/nim/* Docker images
static LOCAL_NIM_FULL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"nvcr\.io/nim/([a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+):([a-zA-Z0-9._-]+)")
        .expect("Invalid LOCAL_NIM_FULL regex")
});

static LOCAL_NIM_NO_TAG: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"nvcr\.io/nim/([a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+)(?:[^:a-zA-Z0-9._-]|$)")
        .expect("Invalid LOCAL_NIM_NO_TAG regex")
});

/// Hosted NIM patterns - matches NVIDIA API endpoints and model references
static HOSTED_ENDPOINT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"https://(?:integrate|ai|build)\.api\.nvidia\.com[^\s"'\)]*"#)
        .expect("Invalid HOSTED_ENDPOINT regex")
});

/// Model assignment pattern - matches model = "xxx" or model: "xxx"
/// Must contain "/" or known prefixes to avoid false positives
static MODEL_ASSIGN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"model\s*[=:]\s*["']((nvidia|meta|mistralai|google|deepseek|stg)/[^"']+|[^"']+/[^"']+)["']"#)
        .expect("Invalid MODEL_ASSIGN regex")
});

static CHATNVIDIA: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"ChatNVIDIA\s*\([^)]*model\s*=\s*["']([^"']+)["']"#)
        .expect("Invalid CHATNVIDIA regex")
});

/// Additional LangChain NVIDIA integrations
static NVIDIA_EMBEDDINGS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"NVIDIAEmbeddings\s*\([^)]*model\s*=\s*["']([^"']+)["']"#)
        .expect("Invalid NVIDIA_EMBEDDINGS regex")
});

static NVIDIA_RERANK: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"NVIDIARerank\s*\([^)]*model\s*=\s*["']([^"']+)["']"#)
        .expect("Invalid NVIDIA_RERANK regex")
});

// ============================================================================
// Source Type Classification
// ============================================================================

/// Determine the source type based on file path
///
/// Files in `.github/workflows/` are classified as ActionsWorkflow,
/// everything else is SourceCode.
pub fn determine_source_type(file_path: &str) -> SourceType {
    let normalized = file_path.replace('\\', "/");
    
    if normalized.contains(".github/workflows/") &&
       (normalized.ends_with(".yml") || normalized.ends_with(".yaml")) {
        SourceType::ActionsWorkflow
    } else {
        SourceType::SourceCode
    }
}

// ============================================================================
// File Filtering
// ============================================================================

/// File extensions to scan
const SCAN_EXTENSIONS: &[&str] = &[
    "py", "yaml", "yml", "sh", "bash", "js", "ts", "jsx", "tsx",
    "dockerfile", "env", "json", "toml", "cfg", "ini", "conf",
];

/// Directory names to skip (matched as path components, not substrings)
const SKIP_DIRS: &[&str] = &[
    "node_modules", "vendor", "__pycache__", ".venv", "venv",
    "target", "build", "dist", ".tox", ".pytest_cache", ".mypy_cache",
    "eggs", ".eggs",
];

/// Check if a file should be scanned based on its name/extension
fn should_scan_file(path: &Path) -> bool {
    let file_name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    
    // Always scan Dockerfiles
    if file_name.to_lowercase().starts_with("dockerfile") {
        return true;
    }
    
    // Check extension
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        return SCAN_EXTENSIONS.contains(&ext.to_lowercase().as_str());
    }
    
    false
}

// ============================================================================
// Extraction Functions
// ============================================================================

/// Extract Local NIM reference from a line
fn extract_local_nim(
    line: &str,
    line_number: usize,
    file_path: &str,
    repository: &str,
) -> Option<LocalNimMatch> {
    // Try full pattern with tag first
    if let Some(caps) = LOCAL_NIM_FULL.captures(line) {
        let namespace_name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let tag = caps.get(2).map(|m| m.as_str()).unwrap_or("latest");
        
        return Some(LocalNimMatch {
            repository: repository.to_string(),
            image_url: format!("nvcr.io/nim/{}", namespace_name),
            tag: tag.to_string(),
            resolved_tag: None,
            file_path: file_path.to_string(),
            line_number,
            match_context: line.trim().to_string(),
        });
    }
    
    // Try pattern without tag
    if let Some(caps) = LOCAL_NIM_NO_TAG.captures(line) {
        let namespace_name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        
        return Some(LocalNimMatch {
            repository: repository.to_string(),
            image_url: format!("nvcr.io/nim/{}", namespace_name),
            tag: "latest".to_string(),
            resolved_tag: None,
            file_path: file_path.to_string(),
            line_number,
            match_context: line.trim().to_string(),
        });
    }
    
    None
}

/// Extract Hosted NIM references from a line
fn extract_hosted_nim(
    line: &str,
    line_number: usize,
    file_path: &str,
    repository: &str,
) -> Vec<HostedNimMatch> {
    let mut matches = Vec::new();
    
    // Extract endpoint URL
    let endpoint = HOSTED_ENDPOINT.find(line).map(|m| m.as_str().to_string());
    
    // Extract model name from various patterns
    let mut model_name: Option<String> = None;
    
    if let Some(caps) = MODEL_ASSIGN.captures(line) {
        model_name = caps.get(1).map(|m| m.as_str().to_string());
    }
    
    if model_name.is_none() {
        if let Some(caps) = CHATNVIDIA.captures(line) {
            model_name = caps.get(1).map(|m| m.as_str().to_string());
        }
    }
    
    if model_name.is_none() {
        if let Some(caps) = NVIDIA_EMBEDDINGS.captures(line) {
            model_name = caps.get(1).map(|m| m.as_str().to_string());
        }
    }
    
    if model_name.is_none() {
        if let Some(caps) = NVIDIA_RERANK.captures(line) {
            model_name = caps.get(1).map(|m| m.as_str().to_string());
        }
    }
    
    // If no explicit model name but we have an endpoint URL, try to extract model from URL path
    // e.g., https://ai.api.nvidia.com/v1/cv/baidu/paddleocr -> baidu/paddleocr
    // e.g., https://ai.api.nvidia.com/v1/cv/nvidia/nemoretriever-page-elements-v2 -> nvidia/nemoretriever-page-elements-v2
    if model_name.is_none() {
        if let Some(ref url) = endpoint {
            model_name = extract_model_from_url(url);
        }
    }
    
    // Only create a match if we found something
    if endpoint.is_some() || model_name.is_some() {
        matches.push(HostedNimMatch {
            repository: repository.to_string(),
            endpoint_url: endpoint,
            model_name,
            file_path: file_path.to_string(),
            line_number,
            match_context: line.trim().to_string(),
            function_id: None,
            status: None,
            container_image: None,
        });
    }
    
    matches
}

/// Extract model name from NVIDIA API URL path
/// 
/// Examples:
/// - https://ai.api.nvidia.com/v1/cv/baidu/paddleocr -> baidu/paddleocr
/// - https://ai.api.nvidia.com/v1/cv/nvidia/nemoretriever-page-elements-v2 -> nvidia/nemoretriever-page-elements-v2
/// - https://integrate.api.nvidia.com/v1 -> None (no model in path)
/// - https://ai.api.nvidia.com/v1/chat/completions -> None (generic endpoint)
fn extract_model_from_url(url: &str) -> Option<String> {
    // Parse URL to get path
    let path = url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    
    // Remove domain
    let path = path.split('/').skip(1).collect::<Vec<_>>().join("/");
    
    // Skip generic endpoints
    if path.is_empty() || 
       path == "v1" || 
       path.ends_with("/chat/completions") ||
       path.ends_with("/embeddings") ||
       path.ends_with("/completions") {
        return None;
    }
    
    // Pattern: v1/{category}/{org}/{model} or v1/{org}/{model}
    let parts: Vec<&str> = path.split('/').collect();
    
    // Look for org/model pattern (last two non-empty segments)
    if parts.len() >= 4 {
        // v1/cv/baidu/paddleocr -> baidu/paddleocr
        // v1/cv/nvidia/nemoretriever-page-elements-v2 -> nvidia/nemoretriever-page-elements-v2
        let org = parts[parts.len() - 2];
        let model = parts[parts.len() - 1];
        
        // Validate org looks like an organization name
        if !org.is_empty() && !model.is_empty() && 
           org != "v1" && org != "chat" && org != "embeddings" {
            return Some(format!("{}/{}", org, model));
        }
    }
    
    None
}

// ============================================================================
// File Scanning
// ============================================================================

/// Scan a single file for NIM references
pub fn scan_file(
    path: &Path,
    repository: &str,
    repo_root: &Path,
) -> (Vec<LocalNimMatch>, Vec<HostedNimMatch>) {
    let mut local_matches = Vec::new();
    let mut hosted_matches = Vec::new();
    
    // Get relative path
    let relative_path = path
        .strip_prefix(repo_root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    
    // Check if this is a YAML file (needs multi-line context)
    let is_yaml = relative_path.ends_with(".yml") || relative_path.ends_with(".yaml");
    
    // Open file and read all lines for context-aware scanning
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read file {}: {}", path.display(), e);
            return (local_matches, hosted_matches);
        }
    };
    
    let lines: Vec<&str> = content.lines().collect();
    
    // Scan line by line
    for (line_num, line) in lines.iter().enumerate() {
        let line_number = line_num + 1; // 1-indexed
        
        // Extract Local NIM
        if let Some(m) = extract_local_nim(line, line_number, &relative_path, repository) {
            debug!("Found Local NIM in {}:{}: {}", relative_path, line_number, m.image_url);
            local_matches.push(m);
        }
        
        // Extract Hosted NIM with multi-line context for YAML files
        let mut hosted = extract_hosted_nim(line, line_number, &relative_path, repository);
        
        // For YAML files, if we found an endpoint but no model_name, look in nearby lines
        if is_yaml {
            for m in &mut hosted {
                if m.model_name.is_none() && m.endpoint_url.is_some() {
                    // Look up to 10 lines before and after for model_name
                    m.model_name = find_model_name_in_context(&lines, line_num, 10);
                    if m.model_name.is_some() {
                        debug!("Found model_name from context: {:?}", m.model_name);
                    }
                }
            }
        }
        
        for m in hosted {
            debug!("Found Hosted NIM in {}:{}: {:?} {:?}",
                   relative_path, line_number, m.endpoint_url, m.model_name);
            hosted_matches.push(m);
        }
    }
    
    (local_matches, hosted_matches)
}

/// Find model_name in surrounding lines (for YAML context)
fn find_model_name_in_context(lines: &[&str], current_line: usize, range: usize) -> Option<String> {
    // Regex pattern for model_name in YAML
    let model_name_re = regex::Regex::new(
        r#"model(?:_name)?\s*[:=]\s*["']?([a-zA-Z0-9_/-]+/[a-zA-Z0-9._-]+)["']?"#
    ).ok()?;
    
    // Search backwards first (model_name usually comes before base_url)
    let start = current_line.saturating_sub(range);
    for i in (start..current_line).rev() {
        if let Some(line) = lines.get(i) {
            if let Some(caps) = model_name_re.captures(line) {
                if let Some(model) = caps.get(1) {
                    return Some(model.as_str().to_string());
                }
            }
        }
    }
    
    // Also search forward in case model comes after
    let end = (current_line + range).min(lines.len());
    for i in (current_line + 1)..end {
        if let Some(line) = lines.get(i) {
            if let Some(caps) = model_name_re.captures(line) {
                if let Some(model) = caps.get(1) {
                    return Some(model.as_str().to_string());
                }
            }
        }
    }
    
    None
}

/// Scan a directory for NIM references
pub fn scan_directory(
    repo_path: &Path,
    repository: &str,
) -> (Vec<LocalNimMatch>, Vec<HostedNimMatch>) {
    let mut all_local: Vec<LocalNimMatch> = Vec::new();
    let mut all_hosted: Vec<HostedNimMatch> = Vec::new();
    
    // Build walker with ignore rules
    let walker = WalkBuilder::new(repo_path)
        .hidden(false)  // Don't skip hidden files (we need .github/)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .build();
    
    // Collect files to scan
    let files: Vec<_> = walker
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter(|entry| {
            let path = entry.path();
            
            // Skip files in excluded directories (match by path component, not substring)
            for component in path.components() {
                if let std::path::Component::Normal(name) = component {
                    if let Some(name_str) = name.to_str() {
                        // Skip .git directory but NOT .github
                        if name_str == ".git" {
                            return false;
                        }
                        // Skip other excluded directories
                        if SKIP_DIRS.contains(&name_str) {
                            return false;
                        }
                    }
                }
            }
            
            should_scan_file(path)
        })
        .map(|entry| entry.into_path())
        .collect();
    
    debug!("Found {} files to scan in {}", files.len(), repo_path.display());
    
    // Scan files in parallel
    let results: Vec<_> = files
        .par_iter()
        .map(|path| scan_file(path, repository, repo_path))
        .collect();
    
    // Aggregate results
    for (local, hosted) in results {
        all_local.extend(local);
        all_hosted.extend(hosted);
    }
    
    (all_local, all_hosted)
}

// ============================================================================
// Result Categorization
// ============================================================================

/// Categorize scan results by source type
pub fn categorize_results(
    local_matches: Vec<LocalNimMatch>,
    hosted_matches: Vec<HostedNimMatch>,
) -> (NimFindings, NimFindings) {
    let mut source_code = NimFindings::new();
    let mut actions_workflow = NimFindings::new();
    
    for m in local_matches {
        match determine_source_type(&m.file_path) {
            SourceType::SourceCode => source_code.local_nim.push(m),
            SourceType::ActionsWorkflow => actions_workflow.local_nim.push(m),
        }
    }
    
    for m in hosted_matches {
        match determine_source_type(&m.file_path) {
            SourceType::SourceCode => source_code.hosted_nim.push(m),
            SourceType::ActionsWorkflow => actions_workflow.hosted_nim.push(m),
        }
    }
    
    (source_code, actions_workflow)
}

/// Deduplicate results based on (repository, file_path, line_number)
pub fn deduplicate_results(findings: &mut NimFindings) {
    use std::collections::HashSet;
    
    // Deduplicate local_nim
    let mut seen: HashSet<(String, String, usize)> = HashSet::new();
    findings.local_nim.retain(|m| {
        let key = (m.repository.clone(), m.file_path.clone(), m.line_number);
        seen.insert(key)
    });
    
    // Deduplicate hosted_nim
    seen.clear();
    findings.hosted_nim.retain(|m| {
        let key = (m.repository.clone(), m.file_path.clone(), m.line_number);
        seen.insert(key)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determine_source_type() {
        assert_eq!(
            determine_source_type("src/main.py"),
            SourceType::SourceCode
        );
        assert_eq!(
            determine_source_type(".github/workflows/deploy.yml"),
            SourceType::ActionsWorkflow
        );
        assert_eq!(
            determine_source_type(".github/workflows/test.yaml"),
            SourceType::ActionsWorkflow
        );
        assert_eq!(
            determine_source_type(".github/actions/test.yml"),
            SourceType::SourceCode  // Not in workflows/
        );
    }

    #[test]
    fn test_extract_local_nim_with_tag() {
        let line = "image: nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2:1.10.0";
        let result = extract_local_nim(line, 1, "docker-compose.yaml", "test/repo");
        
        assert!(result.is_some());
        let m = result.unwrap();
        assert_eq!(m.image_url, "nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2");
        assert_eq!(m.tag, "1.10.0");
    }

    #[test]
    fn test_extract_local_nim_without_tag() {
        let line = "FROM nvcr.io/nim/nvidia/nemo-retriever";
        let result = extract_local_nim(line, 1, "Dockerfile", "test/repo");
        
        assert!(result.is_some());
        let m = result.unwrap();
        assert_eq!(m.image_url, "nvcr.io/nim/nvidia/nemo-retriever");
        assert_eq!(m.tag, "latest");
    }

    #[test]
    fn test_extract_hosted_nim_endpoint() {
        let line = r#"base_url = "https://ai.api.nvidia.com/v1/chat""#;
        let result = extract_hosted_nim(line, 1, "client.py", "test/repo");
        
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].endpoint_url.as_deref(), Some("https://ai.api.nvidia.com/v1/chat"));
    }

    #[test]
    fn test_extract_hosted_nim_model() {
        let line = r#"model = "nvidia/llama-3.1-nemotron-70b-instruct""#;
        let result = extract_hosted_nim(line, 1, "client.py", "test/repo");
        
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].model_name.as_deref(), Some("nvidia/llama-3.1-nemotron-70b-instruct"));
    }

    #[test]
    fn test_extract_hosted_nim_chatnvidia() {
        let line = r#"llm = ChatNVIDIA(model="nvidia/llama-3.1-nemotron")"#;
        let result = extract_hosted_nim(line, 1, "chain.py", "test/repo");
        
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].model_name.as_deref(), Some("nvidia/llama-3.1-nemotron"));
    }

    #[test]
    fn test_should_scan_file() {
        assert!(should_scan_file(Path::new("src/main.py")));
        assert!(should_scan_file(Path::new("docker-compose.yaml")));
        assert!(should_scan_file(Path::new("Dockerfile")));
        assert!(should_scan_file(Path::new("deploy/Dockerfile.prod")));
        assert!(should_scan_file(Path::new("script.sh")));
        
        assert!(!should_scan_file(Path::new("image.png")));
        assert!(!should_scan_file(Path::new("data.csv")));
        // Note: .json files are scanned (package-lock.json would match)
    }

    #[test]
    fn test_categorize_results() {
        let local = vec![
            LocalNimMatch {
                repository: "test".to_string(),
                image_url: "nvcr.io/nim/nvidia/test".to_string(),
                tag: "1.0".to_string(),
                resolved_tag: None,
                file_path: "Dockerfile".to_string(),
                line_number: 1,
                match_context: "FROM nvcr.io/nim/nvidia/test:1.0".to_string(),
            },
            LocalNimMatch {
                repository: "test".to_string(),
                image_url: "nvcr.io/nim/nvidia/test2".to_string(),
                tag: "2.0".to_string(),
                resolved_tag: None,
                file_path: ".github/workflows/deploy.yml".to_string(),
                line_number: 10,
                match_context: "image: nvcr.io/nim/nvidia/test2:2.0".to_string(),
            },
        ];
        
        let hosted = vec![];
        
        let (source_code, actions_workflow) = categorize_results(local, hosted);
        
        assert_eq!(source_code.local_nim.len(), 1);
        assert_eq!(actions_workflow.local_nim.len(), 1);
    }

    #[test]
    fn test_deduplicate_results() {
        let mut findings = NimFindings {
            local_nim: vec![
                LocalNimMatch {
                    repository: "test".to_string(),
                    image_url: "nvcr.io/nim/nvidia/test".to_string(),
                    tag: "1.0".to_string(),
                    resolved_tag: None,
                    file_path: "Dockerfile".to_string(),
                    line_number: 1,
                    match_context: "FROM nvcr.io/nim/nvidia/test:1.0".to_string(),
                },
                LocalNimMatch {
                    repository: "test".to_string(),
                    image_url: "nvcr.io/nim/nvidia/test".to_string(),
                    tag: "1.0".to_string(),
                    resolved_tag: None,
                    file_path: "Dockerfile".to_string(),
                    line_number: 1,  // Same line - duplicate
                    match_context: "FROM nvcr.io/nim/nvidia/test:1.0".to_string(),
                },
            ],
            hosted_nim: vec![],
        };
        
        deduplicate_results(&mut findings);
        assert_eq!(findings.local_nim.len(), 1);
    }
}
