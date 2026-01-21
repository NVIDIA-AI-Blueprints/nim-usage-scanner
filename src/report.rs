//! Report generation module
//!
//! This module handles generating JSON and CSV reports from scan results.

use std::path::Path;
use std::fs::File;
use std::io::Write;
use anyhow::{Context, Result};
use log::info;

use crate::models::ScanReport;

#[cfg(test)]
use crate::models::{LocalNimMatch, HostedNimMatch};

// ============================================================================
// JSON Report Generation
// ============================================================================

/// Generate a JSON report file
pub fn generate_json_report(report: &ScanReport, output_path: &Path) -> Result<()> {
    info!("Generating JSON report: {}", output_path.display());
    
    let json = serde_json::to_string_pretty(report)
        .context("Failed to serialize report to JSON")?;
    
    let mut file = File::create(output_path)
        .with_context(|| format!("Failed to create file: {}", output_path.display()))?;
    
    file.write_all(json.as_bytes())
        .with_context(|| format!("Failed to write to file: {}", output_path.display()))?;
    
    info!("JSON report written to {}", output_path.display());
    Ok(())
}

// ============================================================================
// CSV Report Generation
// ============================================================================

/// Generate a unified CSV report file
pub fn generate_csv_reports(report: &ScanReport, output_dir: &Path) -> Result<()> {
    // Ensure output directory exists
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create output directory: {}", output_dir.display()))?;
    
    let output_path = output_dir.join("report.csv");
    info!("Generating unified CSV report: {}", output_path.display());
    
    let mut writer = csv::Writer::from_path(&output_path)
        .with_context(|| format!("Failed to create CSV file: {}", output_path.display()))?;
    
    // Write header with all columns
    writer.write_record([
        "source_type",      // source_code or actions_workflow
        "nim_type",         // local_nim or hosted_nim
        "repository",
        "file_path",
        "line_number",
        "image_url",        // Local NIM only
        "tag",              // Local NIM only
        "resolved_tag",     // Local NIM only (from NGC API)
        "endpoint_url",     // Hosted NIM only
        "model_name",       // Hosted NIM only
        "function_id",      // Hosted NIM only (from NGC API)
        "status",           // Hosted NIM only (from NGC API)
        "container_image",  // Hosted NIM only (from NGC API)
        "match_context",
    ])?;
    
    // Write source_code local_nim
    for m in &report.source_code.local_nim {
        writer.write_record([
            "source_code",
            "local_nim",
            &m.repository,
            &m.file_path,
            &m.line_number.to_string(),
            &m.image_url,
            &m.tag,
            m.resolved_tag.as_deref().unwrap_or(""),
            "",  // endpoint_url
            "",  // model_name
            "",  // function_id
            "",  // status
            "",  // container_image
            &m.match_context,
        ])?;
    }
    
    // Write source_code hosted_nim
    for m in &report.source_code.hosted_nim {
        writer.write_record([
            "source_code",
            "hosted_nim",
            &m.repository,
            &m.file_path,
            &m.line_number.to_string(),
            "",  // image_url
            "",  // tag
            "",  // resolved_tag
            m.endpoint_url.as_deref().unwrap_or(""),
            m.model_name.as_deref().unwrap_or(""),
            m.function_id.as_deref().unwrap_or(""),
            m.status.as_deref().unwrap_or(""),
            m.container_image.as_deref().unwrap_or(""),
            &m.match_context,
        ])?;
    }
    
    // Write actions_workflow local_nim
    for m in &report.actions_workflow.local_nim {
        writer.write_record([
            "actions_workflow",
            "local_nim",
            &m.repository,
            &m.file_path,
            &m.line_number.to_string(),
            &m.image_url,
            &m.tag,
            m.resolved_tag.as_deref().unwrap_or(""),
            "",  // endpoint_url
            "",  // model_name
            "",  // function_id
            "",  // status
            "",  // container_image
            &m.match_context,
        ])?;
    }
    
    // Write actions_workflow hosted_nim
    for m in &report.actions_workflow.hosted_nim {
        writer.write_record([
            "actions_workflow",
            "hosted_nim",
            &m.repository,
            &m.file_path,
            &m.line_number.to_string(),
            "",  // image_url
            "",  // tag
            "",  // resolved_tag
            m.endpoint_url.as_deref().unwrap_or(""),
            m.model_name.as_deref().unwrap_or(""),
            m.function_id.as_deref().unwrap_or(""),
            m.status.as_deref().unwrap_or(""),
            m.container_image.as_deref().unwrap_or(""),
            &m.match_context,
        ])?;
    }
    
    writer.flush()?;
    info!("CSV report written to {}", output_path.display());
    Ok(())
}


// ============================================================================
// Summary Printing
// ============================================================================

/// Print a summary of the scan results to stdout
pub fn print_summary(report: &ScanReport) {
    println!("\n========================================");
    println!("         NIM Usage Scanner Report       ");
    println!("========================================\n");
    
    println!("Scan Time: {}", report.scan_time);
    println!("Total Repositories: {}", report.total_repos);
    println!();
    
    println!("--- Summary ---");
    println!("Total Local NIM references:  {}", report.summary.total_local_nim);
    println!("Total Hosted NIM references: {}", report.summary.total_hosted_nim);
    println!("Repositories with NIM:       {}", report.summary.repos_with_nim);
    println!();
    
    println!("--- By Source Type ---");
    println!("Source Code:");
    println!("  Local NIM:  {}", report.summary.source_code.local_nim);
    println!("  Hosted NIM: {}", report.summary.source_code.hosted_nim);
    println!();
    println!("Actions Workflow:");
    println!("  Local NIM:  {}", report.summary.actions_workflow.local_nim);
    println!("  Hosted NIM: {}", report.summary.actions_workflow.hosted_nim);
    println!();
    
    // Print some sample findings
    if !report.source_code.local_nim.is_empty() || !report.actions_workflow.local_nim.is_empty() {
        println!("--- Sample Local NIM Findings ---");
        for m in report.source_code.local_nim.iter().take(3) {
            println!("  [source] {}:{} - {}:{}", 
                     m.repository, m.file_path, m.image_url, m.tag);
        }
        for m in report.actions_workflow.local_nim.iter().take(3) {
            println!("  [workflow] {}:{} - {}:{}",
                     m.repository, m.file_path, m.image_url, m.tag);
        }
        println!();
    }
    
    if !report.source_code.hosted_nim.is_empty() || !report.actions_workflow.hosted_nim.is_empty() {
        println!("--- Sample Hosted NIM Findings ---");
        for m in report.source_code.hosted_nim.iter().take(3) {
            println!("  [source] {}:{} - {:?}",
                     m.repository, m.file_path, m.model_name);
        }
        for m in report.actions_workflow.hosted_nim.iter().take(3) {
            println!("  [workflow] {}:{} - {:?}",
                     m.repository, m.file_path, m.model_name);
        }
        println!();
    }
    
    println!("========================================\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::models::NimFindings;

    fn create_test_report() -> ScanReport {
        let source_code = NimFindings {
            local_nim: vec![
                LocalNimMatch {
                    repository: "test/repo".to_string(),
                    image_url: "nvcr.io/nim/nvidia/test".to_string(),
                    tag: "1.0.0".to_string(),
                    resolved_tag: None,
                    file_path: "Dockerfile".to_string(),
                    line_number: 1,
                    match_context: "FROM nvcr.io/nim/nvidia/test:1.0.0".to_string(),
                },
            ],
            hosted_nim: vec![
                HostedNimMatch {
                    repository: "test/repo".to_string(),
                    endpoint_url: Some("https://ai.api.nvidia.com/v1".to_string()),
                    model_name: Some("nvidia/test-model".to_string()),
                    file_path: "src/main.py".to_string(),
                    line_number: 10,
                    match_context: "model=\"nvidia/test-model\"".to_string(),
                    function_id: Some("test-id".to_string()),
                    status: Some("ACTIVE".to_string()),
                    container_image: None,
                },
            ],
        };
        let actions_workflow = NimFindings::default();
        
        ScanReport::new(2, source_code, actions_workflow)
    }

    #[test]
    fn test_generate_json_report() {
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("report.json");
        let report = create_test_report();
        
        let result = generate_json_report(&report, &output_path);
        assert!(result.is_ok());
        assert!(output_path.exists());
        
        // Verify content
        let content = std::fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("nvcr.io/nim/nvidia/test"));
        assert!(content.contains("source_code"));
        assert!(content.contains("actions_workflow"));
    }

    #[test]
    fn test_generate_csv_reports() {
        let temp_dir = TempDir::new().unwrap();
        let report = create_test_report();
        
        let result = generate_csv_reports(&report, temp_dir.path());
        assert!(result.is_ok());
        
        // Verify unified CSV file exists
        let csv_path = temp_dir.path().join("report.csv");
        assert!(csv_path.exists());
        
        // Verify content
        let csv_content = std::fs::read_to_string(&csv_path).unwrap();
        assert!(csv_content.contains("source_type,nim_type,repository"));
        assert!(csv_content.contains("source_code,local_nim"));
        assert!(csv_content.contains("nvcr.io/nim/nvidia/test"));
        assert!(csv_content.contains("source_code,hosted_nim"));
        assert!(csv_content.contains("nvidia/test-model"));
    }
}
