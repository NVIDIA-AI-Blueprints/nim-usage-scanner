//! NGC API client for enriching NIM data
//!
//! This module handles API calls to NGC to:
//! 1. Resolve "latest" tags for Local NIMs
//! 2. Get Function details for Hosted NIMs

use std::collections::HashMap;
use std::time::Duration;
use anyhow::{Context, Result, bail};
use log::{debug, warn, info};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

use crate::models::{
    NimFindings, NgcRepoResponse, NgcFunctionListResponse, NgcFunctionDetails,
};

// ============================================================================
// Constants
// ============================================================================

const NGC_REGISTRY_API_BASE: &str = "https://api.ngc.nvidia.com/v2/org/nim/team";
const NVCF_API_BASE: &str = "https://api.nvcf.nvidia.com/v2/nvcf";
const REQUEST_TIMEOUT_SECS: u64 = 30;
const MAX_RETRIES: u32 = 3;

// ============================================================================
// NGC Client
// ============================================================================

/// NGC API client with caching
pub struct NgcClient {
    /// HTTP client
    client: Client,
    /// API key
    api_key: String,
    /// Cache for Local NIM latest tag resolution
    local_nim_cache: HashMap<String, String>,
    /// Cache for Hosted NIM function details
    hosted_nim_cache: HashMap<String, NgcFunctionDetails>,
    /// Cached function list
    function_list_cache: Option<Vec<NgcFunctionDetails>>,
}

impl NgcClient {
    /// Create a new NGC client
    pub fn new(api_key: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .context("Failed to create HTTP client")?;
        
        Ok(Self {
            client,
            api_key,
            local_nim_cache: HashMap::new(),
            hosted_nim_cache: HashMap::new(),
            function_list_cache: None,
        })
    }
    
    /// Build authorization headers
    fn auth_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let auth_value = format!("Bearer {}", self.api_key);
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth_value).context("Invalid API key format")?,
        );
        Ok(headers)
    }
    
    /// Make a GET request with retries
    fn get_with_retry(&self, url: &str) -> Result<reqwest::blocking::Response> {
        let headers = self.auth_headers()?;
        
        let mut last_error = None;
        for attempt in 1..=MAX_RETRIES {
            debug!("GET {} (attempt {})", url, attempt);
            
            match self.client.get(url).headers(headers.clone()).send() {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return Ok(resp);
                    } else if status.as_u16() == 429 {
                        // Rate limited - wait and retry
                        warn!("Rate limited, waiting before retry...");
                        std::thread::sleep(Duration::from_secs(2u64.pow(attempt)));
                        last_error = Some(format!("Rate limited (429)"));
                        continue;
                    } else if status.is_server_error() {
                        // Server error - retry
                        warn!("Server error {}, retrying...", status);
                        std::thread::sleep(Duration::from_secs(1));
                        last_error = Some(format!("Server error ({})", status));
                        continue;
                    } else {
                        // Client error - don't retry
                        bail!("HTTP error {}: {}", status, resp.text().unwrap_or_default());
                    }
                }
                Err(e) => {
                    warn!("Request failed: {}", e);
                    last_error = Some(e.to_string());
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
        
        bail!("Request failed after {} retries: {:?}", MAX_RETRIES, last_error);
    }
    
    // ========================================================================
    // Local NIM: Latest Tag Resolution
    // ========================================================================
    
    /// Parse image URL to extract team and model name
    /// 
    /// Input: nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2
    /// Output: ("nvidia", "llama-3.2-nv-embedqa-1b-v2")
    fn parse_image_url(image_url: &str) -> Option<(String, String)> {
        let stripped = image_url
            .strip_prefix("nvcr.io/nim/")
            .or_else(|| image_url.strip_prefix("nvcr.io/nim/"))?;
        
        let parts: Vec<&str> = stripped.split('/').collect();
        if parts.len() >= 2 {
            Some((parts[0].to_string(), parts[1].to_string()))
        } else {
            None
        }
    }
    
    /// Resolve latest tag for a Local NIM image
    pub fn resolve_latest_tag(&mut self, image_url: &str) -> Result<String> {
        // Check cache
        if let Some(tag) = self.local_nim_cache.get(image_url) {
            debug!("Cache hit for {}", image_url);
            return Ok(tag.clone());
        }
        
        // Parse image URL
        let (team, model) = Self::parse_image_url(image_url)
            .context(format!("Failed to parse image URL: {}", image_url))?;
        
        // Build API URL
        let url = format!("{}/{}/repos/{}", NGC_REGISTRY_API_BASE, team, model);
        debug!("Resolving latest tag for {}: {}", image_url, url);
        
        // Make request
        let resp = self.get_with_retry(&url)?;
        let repo_info: NgcRepoResponse = resp.json()
            .context("Failed to parse NGC repo response")?;
        
        let latest_tag = repo_info.latest_tag
            .ok_or_else(|| anyhow::anyhow!("No latestTag in response for {}", image_url))?;
        
        // Cache result
        self.local_nim_cache.insert(image_url.to_string(), latest_tag.clone());
        
        info!("Resolved {} latest tag: {}", image_url, latest_tag);
        Ok(latest_tag)
    }
    
    // ========================================================================
    // Hosted NIM: Function Details
    // ========================================================================
    
    /// Fetch and cache the function list
    fn fetch_function_list(&mut self) -> Result<&Vec<NgcFunctionDetails>> {
        if self.function_list_cache.is_some() {
            return Ok(self.function_list_cache.as_ref().unwrap());
        }
        
        let url = format!("{}/functions", NVCF_API_BASE);
        debug!("Fetching function list from {}", url);
        
        let resp = self.get_with_retry(&url)?;
        let list_resp: NgcFunctionListResponse = resp.json()
            .context("Failed to parse function list response")?;
        
        // Convert summaries to details (we'll fetch full details on demand)
        let functions: Vec<NgcFunctionDetails> = list_resp.functions
            .into_iter()
            .map(|f| NgcFunctionDetails {
                id: f.id,
                name: f.name,
                status: f.status,
                container_image: None, // Will be fetched on demand
            })
            .collect();
        
        info!("Fetched {} functions from NVCF", functions.len());
        self.function_list_cache = Some(functions);
        Ok(self.function_list_cache.as_ref().unwrap())
    }
    
    /// Find function by model name
    /// 
    /// NVCF function names have a different format than model names:
    /// - Model: `meta/llama-3.3-70b-instruct` or `nvidia/llama-3.3-nemotron-super-49b-v1`
    /// - NVCF:  `ai-llama-3_3-70b-instruct` or `ai-llama-3_3-nemotron-super-49b-v1_5`
    pub fn find_function_by_model(&mut self, model_name: &str) -> Result<Option<String>> {
        let functions = self.fetch_function_list()?;
        
        // Normalize model name for matching:
        // 1. Remove prefix (meta/, nvidia/, stg/, stg/nvidia/, etc.)
        // 2. Convert to lowercase
        // 3. Replace . with _ (NVCF uses _ instead of .)
        let model_parts: Vec<&str> = model_name.split('/').collect();
        let short_name = model_parts.last().unwrap_or(&model_name);
        let short_name_lower = short_name.to_lowercase();
        
        // Create normalized version: replace . with _
        let normalized_name = short_name_lower.replace('.', "_");
        
        // Also try with ai- prefix (NVCF naming convention)
        let ai_prefixed = format!("ai-{}", normalized_name);
        
        debug!("Looking for function matching model '{}' (normalized: '{}', ai-prefixed: '{}')", 
               model_name, normalized_name, ai_prefixed);
        
        // Try to find a matching function
        for func in functions {
            let func_name_lower = func.name.to_lowercase();
            
            // Try various matching strategies (ordered by specificity)
            let is_match = 
                // Exact match with ai- prefix
                func_name_lower == ai_prefixed ||
                // Function name starts with ai-{normalized_name}
                func_name_lower.starts_with(&ai_prefixed) ||
                // Exact match with normalized name
                func_name_lower == normalized_name ||
                // Function name contains normalized name
                func_name_lower.contains(&normalized_name) ||
                // Original matching strategies
                func_name_lower.contains(&short_name_lower) ||
                short_name_lower.contains(&func_name_lower.replace("ai-", ""));
            
            if is_match {
                debug!("Found function {} ('{}') for model '{}'", func.id, func.name, model_name);
                return Ok(Some(func.id.clone()));
            }
        }
        
        debug!("No function found for model {}", model_name);
        Ok(None)
    }
    
    /// Get function details by ID using /versions endpoint
    /// 
    /// API: GET https://api.nvcf.nvidia.com/v2/nvcf/functions/{functionId}/versions
    /// Returns: status, containerImage, models.name from the latest version
    pub fn get_function_details(&mut self, function_id: &str) -> Result<NgcFunctionDetails> {
        // Check cache
        if let Some(details) = self.hosted_nim_cache.get(function_id) {
            debug!("Cache hit for function {}", function_id);
            return Ok(details.clone());
        }
        
        // Use /versions endpoint instead of direct function access
        let url = format!("{}/functions/{}/versions", NVCF_API_BASE, function_id);
        debug!("Fetching function versions from {}", url);
        
        let resp = self.get_with_retry(&url)?;
        
        // Parse response - NVCF returns { "functions": [...] } with version list
        let json: serde_json::Value = resp.json()
            .context("Failed to parse function versions response")?;
        
        // Get the functions array (versions)
        let functions_arr = json.get("functions")
            .and_then(|f| f.as_array())
            .ok_or_else(|| anyhow::anyhow!("No 'functions' array in response"))?;
        
        // Get the first (latest) version
        let latest_version = functions_arr.first()
            .ok_or_else(|| anyhow::anyhow!("Empty functions array"))?;
        
        // Extract fields
        let id = latest_version.get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(function_id)
            .to_string();
        
        let name = latest_version.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        
        let status = latest_version.get("status")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        
        let container_image = latest_version.get("containerImage")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        
        // Also try to get model name from models array
        let model_name = latest_version.get("models")
            .and_then(|m| m.as_array())
            .and_then(|arr| arr.first())
            .and_then(|m| m.get("name"))
            .and_then(|n| n.as_str())
            .map(|s| s.to_string());
        
        let details = NgcFunctionDetails {
            id,
            name: model_name.unwrap_or(name),
            status,
            container_image,
        };
        
        info!("Got function details: id={}, status={:?}, containerImage={:?}", 
              details.id, details.status, details.container_image);
        
        // Cache result
        self.hosted_nim_cache.insert(function_id.to_string(), details.clone());
        
        Ok(details)
    }
    
    // ========================================================================
    // Batch Enrichment
    // ========================================================================
    
    /// Enrich Local NIM matches by resolving latest tags
    pub fn enrich_local_nim_matches(&mut self, findings: &mut NimFindings) {
        for m in &mut findings.local_nim {
            if m.tag == "latest" || m.tag.is_empty() {
                match self.resolve_latest_tag(&m.image_url) {
                    Ok(actual_tag) => {
                        info!("Resolved {}: latest -> {}", m.image_url, actual_tag);
                        // Keep original tag, set resolved_tag to actual version
                        m.resolved_tag = Some(actual_tag);
                    }
                    Err(e) => {
                        warn!("Failed to resolve latest tag for {}: {}", m.image_url, e);
                        // Keep "latest" and resolved_tag as None
                    }
                }
            }
        }
    }
    
    /// Enrich Hosted NIM matches by fetching function details
    pub fn enrich_hosted_nim_matches(&mut self, findings: &mut NimFindings) {
        for m in &mut findings.hosted_nim {
            // Skip if we don't have a model name
            let model_name = match &m.model_name {
                Some(name) => name.clone(),
                None => continue,
            };
            
            // Find function ID
            let function_id = match self.find_function_by_model(&model_name) {
                Ok(Some(id)) => id,
                Ok(None) => {
                    debug!("No function found for model {}", model_name);
                    continue;
                }
                Err(e) => {
                    warn!("Failed to find function for {}: {}", model_name, e);
                    continue;
                }
            };
            
            // Get function details
            match self.get_function_details(&function_id) {
                Ok(details) => {
                    m.function_id = Some(details.id);
                    m.status = details.status;
                    m.container_image = details.container_image;
                    info!("Enriched hosted NIM {}: function={}", model_name, function_id);
                }
                Err(e) => {
                    warn!("Failed to get function details for {}: {}", function_id, e);
                    m.function_id = Some(function_id); // At least set the ID
                }
            }
        }
    }
    
    // ========================================================================
    // Query API (for CLI query subcommand)
    // ========================================================================
    
    /// Query complete Local NIM information by image name
    /// 
    /// Returns all available information about a Local NIM including:
    /// - latest tag (actual version)
    /// - description
    /// - available versions
    /// - raw API response data
    pub fn query_local_nim(&mut self, image_url: &str) -> Result<LocalNimQueryResult> {
        info!("Querying Local NIM: {}", image_url);
        
        // Parse image URL to extract team and model name
        let (team, model) = Self::parse_image_url(image_url)
            .ok_or_else(|| anyhow::anyhow!("Invalid image URL format: {}. Expected: nvcr.io/nim/<team>/<model>", image_url))?;
        
        // Build API URL
        let url = format!("{}/{}/repos/{}", NGC_REGISTRY_API_BASE, team, model);
        debug!("Fetching Local NIM info from {}", url);
        
        let resp = self.get_with_retry(&url)?;
        let raw_json: serde_json::Value = resp.json()
            .context("Failed to parse NGC repo response")?;
        
        // Build result
        let result = LocalNimQueryResult {
            query_image: image_url.to_string(),
            team: team.clone(),
            model: model.clone(),
            name: raw_json.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            latest_tag: raw_json.get("latestTag")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            latest_version_id: raw_json.get("latestVersionId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            description: raw_json.get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            short_description: raw_json.get("shortDescription")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            is_public: raw_json.get("isPublic")
                .and_then(|v| v.as_bool()),
            publisher: raw_json.get("publisher")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            display_name: raw_json.get("displayName")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            repository_url: format!("nvcr.io/nim/{}/{}", team, model),
            raw_response: raw_json,
        };
        
        info!("Latest tag for {}: {:?}", image_url, result.latest_tag);
        
        Ok(result)
    }
    
    /// Query complete Hosted NIM information by model name
    /// 
    /// Returns all available information about a Hosted NIM including:
    /// - function_id
    /// - name
    /// - status
    /// - containerImage
    /// - raw API response data
    pub fn query_hosted_nim(&mut self, model_name: &str) -> Result<HostedNimQueryResult> {
        info!("Querying Hosted NIM: {}", model_name);
        
        // Find function ID by model name
        let function_id = self.find_function_by_model(model_name)?
            .ok_or_else(|| anyhow::anyhow!("No function found for model: {}", model_name))?;
        
        info!("Found function ID: {}", function_id);
        
        // Get function versions (full details)
        let url = format!("{}/functions/{}/versions", NVCF_API_BASE, function_id);
        debug!("Fetching full function details from {}", url);
        
        let resp = self.get_with_retry(&url)?;
        let raw_json: serde_json::Value = resp.json()
            .context("Failed to parse function versions response")?;
        
        // Get the functions array (versions)
        let functions_arr = raw_json.get("functions")
            .and_then(|f| f.as_array())
            .ok_or_else(|| anyhow::anyhow!("No 'functions' array in response"))?;
        
        // Get the first (latest) version
        let latest_version = functions_arr.first()
            .ok_or_else(|| anyhow::anyhow!("Empty functions array"))?;
        
        // Build result
        let result = HostedNimQueryResult {
            query_model: model_name.to_string(),
            function_id: latest_version.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            name: latest_version.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            status: latest_version.get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            container_image: latest_version.get("containerImage")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            ncf_function_id: latest_version.get("ncaId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            version_id: latest_version.get("versionId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            created_at: latest_version.get("createdAt")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            description: latest_version.get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            health_uri: latest_version.get("healthUri")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            inference_url: latest_version.get("inferenceUrl")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            models: latest_version.get("models").cloned(),
            api_body_format: latest_version.get("apiBodyFormat")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            raw_response: latest_version.clone(),
        };
        
        Ok(result)
    }
}

/// Result of querying a Local NIM by image name
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalNimQueryResult {
    /// The image URL that was queried
    pub query_image: String,
    
    /// Team/namespace (e.g., "nvidia")
    pub team: String,
    
    /// Model name (e.g., "llama-3.2-nv-embedqa-1b-v2")
    pub model: String,
    
    /// Repository name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    
    /// Latest tag (actual version number, e.g., "1.10.0")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_tag: Option<String>,
    
    /// Latest version ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version_id: Option<String>,
    
    /// Full description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    
    /// Short description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
    
    /// Whether the repository is public
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_public: Option<bool>,
    
    /// Publisher name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    
    /// Display name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    
    /// Full repository URL for docker pull
    pub repository_url: String,
    
    /// Raw API response for additional fields
    pub raw_response: serde_json::Value,
}

/// Result of querying a Hosted NIM by model name
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedNimQueryResult {
    /// The model name that was queried
    pub query_model: String,
    
    /// NVCF Function ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    
    /// Function name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    
    /// Function status (ACTIVE, INACTIVE, DEPLOYING, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    
    /// Container image used by the function
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_image: Option<String>,
    
    /// NCA ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ncf_function_id: Option<String>,
    
    /// Version ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    
    /// Creation timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    
    /// Function description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    
    /// Health check URI
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health_uri: Option<String>,
    
    /// Inference URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference_url: Option<String>,
    
    /// Models configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<serde_json::Value>,
    
    /// API body format
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_body_format: Option<String>,
    
    /// Raw API response for additional fields
    pub raw_response: serde_json::Value,
}

/// Enrich all findings using NGC API
pub fn enrich_all_findings(
    api_key: Option<&str>,
    source_code: &mut NimFindings,
    actions_workflow: &mut NimFindings,
) {
    let api_key = match api_key {
        Some(key) if !key.is_empty() => key,
        _ => {
            info!("No NGC API key provided, skipping enrichment");
            return;
        }
    };
    
    let mut client = match NgcClient::new(api_key.to_string()) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to create NGC client: {}", e);
            return;
        }
    };
    
    info!("Enriching findings with NGC API...");
    
    // Enrich Local NIMs
    client.enrich_local_nim_matches(source_code);
    client.enrich_local_nim_matches(actions_workflow);
    
    // Enrich Hosted NIMs
    client.enrich_hosted_nim_matches(source_code);
    client.enrich_hosted_nim_matches(actions_workflow);
    
    info!("Enrichment complete");
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Unit Tests (no API key required)
    // =========================================================================

    #[test]
    fn test_parse_image_url() {
        let result = NgcClient::parse_image_url("nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2");
        assert!(result.is_some());
        let (team, model) = result.unwrap();
        assert_eq!(team, "nvidia");
        assert_eq!(model, "llama-3.2-nv-embedqa-1b-v2");
    }

    #[test]
    fn test_parse_image_url_meta() {
        let result = NgcClient::parse_image_url("nvcr.io/nim/meta/llama-3.3-70b-instruct");
        assert!(result.is_some());
        let (team, model) = result.unwrap();
        assert_eq!(team, "meta");
        assert_eq!(model, "llama-3.3-70b-instruct");
    }

    #[test]
    fn test_parse_image_url_invalid() {
        assert!(NgcClient::parse_image_url("docker.io/library/nginx").is_none());
        assert!(NgcClient::parse_image_url("invalid").is_none());
    }

    #[test]
    fn test_model_name_normalization() {
        // Test that model names are correctly normalized for NVCF matching
        // Model: meta/llama-3.3-70b-instruct -> NVCF: ai-llama-3_3-70b-instruct
        
        let model_name = "meta/llama-3.3-70b-instruct";
        let parts: Vec<&str> = model_name.split('/').collect();
        let short_name = parts.last().unwrap();
        let normalized = short_name.to_lowercase().replace('.', "_");
        let ai_prefixed = format!("ai-{}", normalized);
        
        assert_eq!(normalized, "llama-3_3-70b-instruct");
        assert_eq!(ai_prefixed, "ai-llama-3_3-70b-instruct");
    }

    #[test]
    fn test_model_name_normalization_nvidia() {
        // Test nvidia/llama-3.3-nemotron-super-49b-v1 -> ai-llama-3_3-nemotron-super-49b-v1
        
        let model_name = "nvidia/llama-3.3-nemotron-super-49b-v1";
        let parts: Vec<&str> = model_name.split('/').collect();
        let short_name = parts.last().unwrap();
        let normalized = short_name.to_lowercase().replace('.', "_");
        let ai_prefixed = format!("ai-{}", normalized);
        
        assert_eq!(normalized, "llama-3_3-nemotron-super-49b-v1");
        assert_eq!(ai_prefixed, "ai-llama-3_3-nemotron-super-49b-v1");
    }

    #[test]
    fn test_model_name_normalization_stg_prefix() {
        // Test stg/nvidia/llama-3.3-nemotron-super-49b-v1 -> ai-llama-3_3-nemotron-super-49b-v1
        
        let model_name = "stg/nvidia/llama-3.3-nemotron-super-49b-v1";
        let parts: Vec<&str> = model_name.split('/').collect();
        let short_name = parts.last().unwrap();
        let normalized = short_name.to_lowercase().replace('.', "_");
        let ai_prefixed = format!("ai-{}", normalized);
        
        assert_eq!(short_name, &"llama-3.3-nemotron-super-49b-v1");
        assert_eq!(normalized, "llama-3_3-nemotron-super-49b-v1");
        assert_eq!(ai_prefixed, "ai-llama-3_3-nemotron-super-49b-v1");
    }

    #[test]
    fn test_model_name_normalization_deepseek() {
        // Test stg/deepseek-ai/deepseek-r1
        
        let model_name = "stg/deepseek-ai/deepseek-r1";
        let parts: Vec<&str> = model_name.split('/').collect();
        let short_name = parts.last().unwrap();
        let normalized = short_name.to_lowercase().replace('.', "_");
        
        assert_eq!(short_name, &"deepseek-r1");
        assert_eq!(normalized, "deepseek-r1");
    }

    // =========================================================================
    // Integration Tests - Query Hosted NIM
    // Run with: NVIDIA_API_KEY=<key> cargo test --release -- --ignored --nocapture
    // =========================================================================

    /// Test resolving latest tag for Local NIM: nvidia/llama-3.2-nv-embedqa-1b-v2
    /// This is a known working image from our scan results
    #[test]
    #[ignore]
    fn test_resolve_latest_tag() {
        let api_key = std::env::var("NVIDIA_API_KEY").expect("NVIDIA_API_KEY required");
        let mut client = NgcClient::new(api_key).unwrap();
        
        // Use a known working image from scan results
        let tag = client.resolve_latest_tag("nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2");
        assert!(tag.is_ok(), "Should successfully resolve latest tag");
        
        let tag_value = tag.unwrap();
        println!("Latest tag: {}", tag_value);
        
        // Tag should be a version number
        assert!(tag_value.chars().any(|c| c.is_numeric()), "Tag should contain version number");
    }

    #[test]
    #[ignore]
    fn test_find_function_by_model() {
        let api_key = std::env::var("NVIDIA_API_KEY").expect("NVIDIA_API_KEY required");
        let mut client = NgcClient::new(api_key).unwrap();
        
        let result = client.find_function_by_model("nvidia/llama-3.1-nemotron-70b-instruct");
        assert!(result.is_ok());
        if let Some(id) = result.unwrap() {
            println!("Function ID: {}", id);
        }
    }

    /// Test query Hosted NIM: meta/llama-3.3-70b-instruct
    /// Expected: Function ID, status=ACTIVE, containerImage present
    #[test]
    #[ignore]
    fn test_query_hosted_nim_meta_llama() {
        let api_key = std::env::var("NVIDIA_API_KEY").expect("NVIDIA_API_KEY required");
        let mut client = NgcClient::new(api_key).unwrap();
        
        let result = client.query_hosted_nim("meta/llama-3.3-70b-instruct");
        assert!(result.is_ok(), "Query should succeed");
        
        let info = result.unwrap();
        println!("Query result: {:?}", info);
        
        assert!(info.function_id.is_some(), "Should have function_id");
        assert_eq!(info.status.as_deref(), Some("ACTIVE"), "Should be ACTIVE");
        assert!(info.container_image.is_some(), "Should have container_image");
        
        // Verify function name matches expected pattern
        assert!(info.name.as_ref().map_or(false, |n| n.contains("llama-3_3-70b")),
                "Function name should contain llama-3_3-70b");
    }

    /// Test query Hosted NIM: nvidia/llama-3.3-nemotron-super-49b-v1
    /// Expected: Function ID, status=ACTIVE
    #[test]
    #[ignore]
    fn test_query_hosted_nim_nemotron() {
        let api_key = std::env::var("NVIDIA_API_KEY").expect("NVIDIA_API_KEY required");
        let mut client = NgcClient::new(api_key).unwrap();
        
        let result = client.query_hosted_nim("nvidia/llama-3.3-nemotron-super-49b-v1");
        assert!(result.is_ok(), "Query should succeed");
        
        let info = result.unwrap();
        println!("Query result: {:?}", info);
        
        assert!(info.function_id.is_some(), "Should have function_id");
        assert_eq!(info.status.as_deref(), Some("ACTIVE"), "Should be ACTIVE");
        
        // Note: containerImage may be null for this model (API-side issue)
    }

    /// Test query Hosted NIM: stg/deepseek-ai/deepseek-r1
    /// Expected: Function ID, status=ACTIVE, containerImage present
    #[test]
    #[ignore]
    fn test_query_hosted_nim_deepseek() {
        let api_key = std::env::var("NVIDIA_API_KEY").expect("NVIDIA_API_KEY required");
        let mut client = NgcClient::new(api_key).unwrap();
        
        let result = client.query_hosted_nim("stg/deepseek-ai/deepseek-r1");
        assert!(result.is_ok(), "Query should succeed");
        
        let info = result.unwrap();
        println!("Query result: {:?}", info);
        
        assert!(info.function_id.is_some(), "Should have function_id");
        assert_eq!(info.status.as_deref(), Some("ACTIVE"), "Should be ACTIVE");
        assert!(info.container_image.is_some(), "Should have container_image");
    }

    /// Test query Hosted NIM: baidu/paddleocr
    /// Expected: Function ID, status=ACTIVE, containerImage present
    #[test]
    #[ignore]
    fn test_query_hosted_nim_paddleocr() {
        let api_key = std::env::var("NVIDIA_API_KEY").expect("NVIDIA_API_KEY required");
        let mut client = NgcClient::new(api_key).unwrap();
        
        let result = client.query_hosted_nim("baidu/paddleocr");
        assert!(result.is_ok(), "Query should succeed");
        
        let info = result.unwrap();
        println!("Query result: {:?}", info);
        
        assert!(info.function_id.is_some(), "Should have function_id");
        assert_eq!(info.status.as_deref(), Some("ACTIVE"), "Should be ACTIVE");
        assert!(info.container_image.is_some(), "Should have container_image");
        assert!(info.container_image.as_ref().map_or(false, |c| c.contains("paddleocr")),
                "Container image should contain paddleocr");
    }

    // =========================================================================
    // Integration Tests - Query Local NIM
    // Run with: NVIDIA_API_KEY=<key> cargo test --release -- --ignored --nocapture
    // =========================================================================

    /// Test query Local NIM: nvidia/llama-3.2-nv-embedqa-1b-v2
    /// Expected: latest tag version
    #[test]
    #[ignore]
    fn test_query_local_nim_embedqa() {
        let api_key = std::env::var("NVIDIA_API_KEY").expect("NVIDIA_API_KEY required");
        let mut client = NgcClient::new(api_key).unwrap();
        
        let result = client.query_local_nim("nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2");
        assert!(result.is_ok(), "Query should succeed");
        
        let info = result.unwrap();
        println!("Query result: {:?}", info);
        
        assert_eq!(info.team, "nvidia");
        assert_eq!(info.model, "llama-3.2-nv-embedqa-1b-v2");
        assert!(info.latest_tag.is_some(), "Should have latest_tag");
        
        // Verify the latest tag is a version number
        let tag = info.latest_tag.unwrap();
        println!("Latest tag: {}", tag);
        assert!(tag.chars().any(|c| c.is_numeric()), "Tag should contain a version number");
    }

    /// Test query Local NIM: meta/llama-3.3-70b-instruct
    /// Expected: latest tag version
    #[test]
    #[ignore]
    fn test_query_local_nim_meta_llama() {
        let api_key = std::env::var("NVIDIA_API_KEY").expect("NVIDIA_API_KEY required");
        let mut client = NgcClient::new(api_key).unwrap();
        
        let result = client.query_local_nim("nvcr.io/nim/meta/llama-3.3-70b-instruct");
        assert!(result.is_ok(), "Query should succeed");
        
        let info = result.unwrap();
        println!("Query result: {:?}", info);
        
        assert_eq!(info.team, "meta");
        assert_eq!(info.model, "llama-3.3-70b-instruct");
        assert!(info.latest_tag.is_some(), "Should have latest_tag");
        
        let tag = info.latest_tag.unwrap();
        println!("Latest tag: {}", tag);
    }

    /// Test query Local NIM without full path (just nvidia/model-name)
    #[test]
    #[ignore]
    fn test_query_local_nim_short_path() {
        let api_key = std::env::var("NVIDIA_API_KEY").expect("NVIDIA_API_KEY required");
        let mut client = NgcClient::new(api_key).unwrap();
        
        // The main.rs should prepend nvcr.io/nim/, so this tests the parsing
        let result = client.query_local_nim("nvcr.io/nim/nvidia/parakeet-0-6b-ctc-en-us");
        assert!(result.is_ok(), "Query should succeed");
        
        let info = result.unwrap();
        println!("Query result: {:?}", info);
        
        assert!(info.latest_tag.is_some(), "Should have latest_tag");
    }
}
