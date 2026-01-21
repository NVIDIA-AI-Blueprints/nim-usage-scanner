# NIM Usage Scanner Implementation Guide

## 1. Project Initialization

### 1.1 Create Rust Project

Execute `cargo init` in the `nim-usage-scanner` directory to create a binary project named `nim-usage-scanner`.

### 1.2 Configure Cargo.toml

Add the following dependencies:

| Dependency | Version | Purpose |
|------------|---------|---------|
| `clap` | 4.x | Command line argument parsing, enable `derive` feature |
| `serde` | 1.x | Serialization framework, enable `derive` feature |
| `serde_json` | 1.x | JSON output |
| `serde_yaml` | 0.9.x | YAML configuration parsing |
| `regex` | 1.x | Regular expression matching |
| `ignore` | 0.4.x | Smart file traversal (ripgrep core library) |
| `rayon` | 1.x | Parallel processing |
| `reqwest` | 0.12.x | HTTP client, enable `blocking` and `json` features |
| `csv` | 1.x | CSV output |
| `chrono` | 0.4.x | Time handling, enable `serde` feature |
| `tempfile` | 3.x | Temporary directory management |
| `anyhow` | 1.x | Error handling |
| `log` | 0.4.x | Logging interface |
| `env_logger` | 0.11.x | Logging implementation |

### 1.3 Create Directory Structure

```
nim-usage-scanner/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI entry
│   ├── config.rs         # Configuration loading
│   ├── git_ops.rs        # Git operations
│   ├── scanner.rs        # Scanning logic
│   ├── ngc_api.rs        # NGC API client
│   ├── report.rs         # Report generation
│   └── models.rs         # Data models
├── config/
│   └── repos.yaml        # Configuration file example
└── docs/
    └── zh/
        ├── architecture-design.md
        └── implementation-guide.md
```

---

## 2. Module Implementation Order

Based on dependencies, the recommended implementation order is:

```
┌─────────────────────────────────────────────────────────────────┐
│  Phase 1: Foundation Modules                                     │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐         │
│  │  models.rs  │ ─▶ │  config.rs  │ ─▶ │  git_ops.rs │         │
│  │ (Data Structs) │  │(Config Parse)│   │(Repo Clone) │         │
│  └─────────────┘    └─────────────┘    └─────────────┘         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 2: Core Scanning                                          │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                      scanner.rs                          │    │
│  │  • File traversal logic                                  │    │
│  │  • Local NIM regex matching                              │    │
│  │  • Hosted NIM regex matching                             │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 3: API Integration                                        │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                      ngc_api.rs                          │    │
│  │  • NGC API authentication                                │    │
│  │  • Function queries                                      │    │
│  │  • Result caching                                        │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 4: Output and Integration                                 │
│  ┌─────────────┐    ┌─────────────┐                            │
│  │  report.rs  │ ─▶ │  main.rs    │                            │
│  │(Report Gen) │    │(CLI Integ)  │                            │
│  └─────────────┘    └─────────────┘                            │
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. Phase 1: Foundation Module Implementation

### 3.1 models.rs - Data Models

**Task 3.1.1: Define Configuration Data Structures**

Define the following structs for parsing `repos.yaml`:

| Struct | Fields | Description |
|--------|--------|-------------|
| `Config` | `version`, `defaults`, `repos` | Top-level configuration |
| `Defaults` | `branch`, `depth` | Default value configuration |
| `RepoConfig` | `name`, `url`, `branch`, `depth`, `enabled` | Single repository configuration |

All structs must implement the `Deserialize` trait.

**Task 3.1.2: Define Source Type Enum**

Define `SourceType` enum for internal classification:

| Enum Value | Description | Condition |
|------------|-------------|-----------|
| `SourceCode` | Regular source code | File path doesn't match `.github/workflows/*.yml` |
| `ActionsWorkflow` | Actions Workflow | File path matches `.github/workflows/*.yml` or `.github/workflows/*.yaml` |

**Task 3.1.3: Define Scan Result Data Structures**

Define the following structs for storing scan results:

| Struct | Fields | Description |
|--------|--------|-------------|
| `LocalNimMatch` | `repository`, `image_url`, `tag`, `resolved_tag`, `file_path`, `line_number`, `match_context` | Local NIM match result (`resolved_tag` is actual version resolved by NGC API) |
| `HostedNimMatch` | `repository`, `endpoint_url`, `model_name`, `file_path`, `line_number`, `match_context`, `function_id`, `status`, `container_image` | Hosted NIM match result |
| `NimFindings` | `local_nim`, `hosted_nim` | NIM result set for a source type |
| `ScanReport` | `scan_time`, `total_repos`, `source_code`, `actions_workflow`, `aggregated`, `summary` | Complete report (top-level by source, with aggregated view) |
| `Summary` | `total_local_nim`, `total_hosted_nim`, `repos_with_nim`, `source_code`, `actions_workflow` | Statistical summary (with category stats) |
| `CategorySummary` | `local_nim`, `hosted_nim` | Statistics for a single source type |

**Data Structure Hierarchy**:

```
ScanReport
├── source_code: NimFindings
│   ├── local_nim: Vec<LocalNimMatch>
│   └── hosted_nim: Vec<HostedNimMatch>
├── actions_workflow: NimFindings
│   ├── local_nim: Vec<LocalNimMatch>
│   └── hosted_nim: Vec<HostedNimMatch>
├── aggregated: AggregatedFindings
│   ├── local_nim: Vec<AggregatedLocalNim>  // Grouped by image_url+tag
│   │   └── locations: Vec<NimLocation>     // All occurrence locations
│   └── hosted_nim: Vec<AggregatedHostedNim>  // Grouped by model_name
│       └── locations: Vec<NimLocation>
└── summary: Summary
    ├── source_code: CategorySummary
    └── actions_workflow: CategorySummary
```

All structs must implement the `Serialize` trait.

**Task 3.1.4: Define NGC API Response Data Structures**

Define the following structs for parsing NGC API responses:

| Struct | Purpose |
|--------|---------|
| `NgcRepoResponse` | Container Registry API response (for latest tag resolution) |
| `NgcFunctionListResponse` | Function list API response |
| `NgcFunctionDetails` | Function details |

### 3.2 config.rs - Configuration Loading

**Task 3.2.1: Implement Configuration File Reading**

Implement `load_config` function:
- Input: Configuration file path
- Output: `Result<Config>`
- Functionality: Read YAML file, parse into `Config` struct

**Task 3.2.2: Implement Configuration Validation**

Implement `validate_config` function:
- Check all repository URL formats are correct (start with `https://` or `git@`)
- Check repository names are unique
- Return list of validation errors

**Task 3.2.3: Implement Default Value Merging**

Implement `apply_defaults` function:
- For each repository config, if `branch` or `depth` is empty, use values from `defaults`
- If `defaults` is also empty, use hardcoded defaults (branch: "main", depth: 1)

### 3.3 git_ops.rs - Git Operations

**Task 3.3.1: Implement Repository Cloning**

Implement `clone_repo` function:
- Input: `RepoConfig`, target directory path
- Output: `Result<PathBuf>` (cloned directory path)
- Functionality:
  1. Build git clone command
  2. Add `--depth` parameter
  3. Add `--branch` parameter (if specified)
  4. Execute command and check return code
  5. Return cloned directory path

**Task 3.3.2: Implement Batch Cloning**

Implement `clone_all_repos` function:
- Input: Repository config list, work directory
- Output: `Vec<(RepoConfig, Result<PathBuf>)>`
- Functionality:
  1. Create work directory (if not exists)
  2. Use rayon for parallel cloning of all repositories
  3. Collect clone results for each repository (success or failure)

**Task 3.3.3: Implement Directory Cleanup**

Implement `cleanup_repos` function:
- Input: Work directory path
- Output: `Result<()>`
- Functionality: Recursively delete work directory

---

## 4. Phase 2: Core Scanning Implementation

### 4.1 scanner.rs - Scanning Logic

**Task 4.1.1: Define Scanning Regular Expressions**

Create two sets of precompiled regular expressions:

**Local NIM Regex**:

| Pattern ID | Match Target | Capture Groups |
|------------|--------------|----------------|
| `LOCAL_FULL` | `nvcr.io/nim/namespace/name:tag` | 1: namespace/name, 2: tag |
| `LOCAL_NO_TAG` | `nvcr.io/nim/namespace/name` | 1: namespace/name |

**Hosted NIM Regex**:

| Pattern ID | Match Target | Capture Groups |
|------------|--------------|----------------|
| `ENDPOINT_URL` | `https://*.api.nvidia.com/*` | 1: complete URL |
| `MODEL_ASSIGN` | `model = "xxx"` or `model: "xxx"` | 1: model name |
| `CHATNVIDIA` | `ChatNVIDIA(model="xxx")` | 1: model name |

Use `lazy_static` or `once_cell` to create regex objects at compile time.

**Task 4.1.2: Implement File Traversal**

Implement `scan_directory` function:
- Input: Directory path, repository name
- Output: `(Vec<LocalNimMatch>, Vec<HostedNimMatch>)`
- Functionality:
  1. Use `ignore` crate's `WalkBuilder` for directory traversal
  2. Automatically respect `.gitignore` rules
  3. Add custom ignore rules (node_modules, vendor, etc.)
  4. Filter by file extensions (only process .py, .yaml, .yml, .sh, Dockerfile, etc.)
  5. Call scan function for each file

**Task 4.1.3: Implement Single File Scanning**

Implement `scan_file` function:
- Input: File path, repository name
- Output: `(Vec<LocalNimMatch>, Vec<HostedNimMatch>)`
- Functionality:
  1. Read file content
  2. Iterate line by line, record line numbers
  3. Apply all regular expressions to each line
  4. Extract match results, build Match objects
  5. Return all matches

**Task 4.1.4: Implement Source Type Determination**

Implement `determine_source_type` function:
- Input: File relative path
- Output: `SourceType`
- Functionality:
  1. Check if path matches `.github/workflows/*.yml` or `.github/workflows/*.yaml`
  2. If matches, return `SourceType::Workflow`
  3. Otherwise return `SourceType::Source`

**Determination Logic**:
```
Path starts with ".github/workflows/"
  AND
  (Path ends with ".yml" OR Path ends with ".yaml")
```

**Task 4.1.5: Implement Local NIM Extraction**

Implement `extract_local_nim` function:
- Input: Line content, line number, file path, repository name
- Output: `Option<LocalNimMatch>`
- Functionality:
  1. Try to match `LOCAL_FULL` regex
  2. If matches, extract image_url and tag
  3. If no match, try `LOCAL_NO_TAG`, set tag to "latest"
  4. Build and return `LocalNimMatch`

**Task 4.1.6: Implement Hosted NIM Extraction**

Implement `extract_hosted_nim` function:
- Input: Line content, line number, file path, repository name
- Output: `Vec<HostedNimMatch>` (one line may have multiple matches)
- Functionality:
  1. Try to match `ENDPOINT_URL` regex, extract endpoint
  2. Try to match `MODEL_ASSIGN` regex, extract model
  3. Try to match `CHATNVIDIA` regex, extract model
  4. Merge endpoint and model from same line
  5. Build and return `HostedNimMatch` list

**Task 4.1.7: Implement Scan Result Classification**

Implement `categorize_results` function:
- Input: `Vec<LocalNimMatch>`, `Vec<HostedNimMatch>`
- Output: `(NimFindings, NimFindings)` (source_code, actions_workflow)
- Functionality:
  1. Iterate all LocalNimMatch, call `determine_source_type` to determine source
  2. Iterate all HostedNimMatch, call `determine_source_type` to determine source
  3. Place in source_code or actions_workflow NimFindings respectively
  4. Return two classified result sets

**Task 4.1.8: Implement Scan Result Deduplication**

Implement `deduplicate_results` function:
- Input: `Vec<LocalNimMatch>`, `Vec<HostedNimMatch>`
- Output: Deduplicated lists
- Functionality:
  1. Deduplicate by (repository, file_path, line_number)
  2. Keep first occurrence of each match

---

## 5. Phase 3: NGC API Integration

### 5.1 ngc_api.rs - NGC API Client

**Task 5.1.1: Implement API Client Structure**

Create `NgcClient` struct:
- Fields:
  - `api_key`: String
  - `client`: reqwest blocking Client
  - `local_nim_cache`: HashMap<String, String> (cache latest tag resolution results)
  - `hosted_nim_cache`: HashMap<String, NgcFunctionDetails> (cache Function details)
- Functionality: Manage API authentication and request caching

**Task 5.1.2: Implement Local NIM Latest Tag Resolution**

Implement `NgcClient::resolve_latest_tag` method:
- Input: image_url (e.g., "nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2")
- Output: `Result<String>` (actual version number)
- Functionality:
  1. Check cache, return immediately if hit
  2. Parse team and model-name from image_url
  3. Call NGC Container Registry API
  4. Extract `latestTag` field from response
  5. Write to cache
  6. Return actual version number

**NGC Container Registry API Endpoint**:
```
GET https://api.ngc.nvidia.com/v2/org/nim/team/{team}/repos/{model-name}
Authorization: Bearer <api_key>
```

**URL Construction Rules**:
- Input: `nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2`
- team = `nvidia`
- model-name = `llama-3.2-nv-embedqa-1b-v2`
- API URL = `https://api.ngc.nvidia.com/v2/org/nim/team/nvidia/repos/llama-3.2-nv-embedqa-1b-v2`

**Response Parsing**:
- Extract `latestTag` field as actual version number
- Return error if field doesn't exist

**Task 5.1.3: Implement Function Search**

Implement `NgcClient::find_function_by_model` method:
- Input: model name (e.g., "nvidia/llama-3.1-nemotron")
- Output: `Option<String>` (Function ID)
- Functionality:
  1. Call NGC Functions List API
  2. Search for functions where name or description contains model name
  3. Return matching Function ID

**NGC Functions List API Endpoint**:
```
GET https://api.nvcf.nvidia.com/v2/nvcf/functions
Authorization: Bearer <api_key>
```

**Task 5.1.4: Implement Function Details Retrieval**

Implement `NgcClient::get_function_details` method:
- Input: Function ID
- Output: `Result<NgcFunctionDetails>`
- Functionality:
  1. Check cache, return immediately if hit
  2. Call NGC Function Details API
  3. Parse response, extract status, containerImage, etc.
  4. Write to cache
  5. Return result

**NGC Function Versions API Endpoint** (⚠️ Must use `/versions` endpoint):
```
GET https://api.nvcf.nvidia.com/v2/nvcf/functions/{function_id}/versions
Authorization: Bearer <api_key>
```

> **Important**: Directly accessing `/v2/nvcf/functions/{id}` returns 404. Must use `/versions` endpoint for details.

**Response Structure**:
```json
{
  "functions": [
    {
      "id": "b6429d64-...",
      "name": "llama-3.1-nemotron-70b-instruct",
      "status": "ACTIVE",
      "containerImage": "nvcr.io/nim/...",
      "models": [{ "name": "model-name", "version": "1.0" }]
    }
  ]
}
```

**Response Field Extraction** (take `functions[0]` for latest version):

| Response Field Path | Output Field |
|--------------------|--------------|
| `functions[0].id` | `function_id` |
| `functions[0].status` | `status` |
| `functions[0].name` or `functions[0].models[0].name` | `model_name` |
| `functions[0].containerImage` | `container_image` |

**Task 5.1.5: Implement Local NIM Batch Enrichment**

Implement `enrich_local_nim_matches` function:
- Input: `&mut Vec<LocalNimMatch>`, `&NgcClient`
- Output: None (modify in place)
- Functionality:
  1. Filter records with tag "latest" or empty
  2. Call `resolve_latest_tag` for each record
  3. **Set `resolved_tag` field to actual version number** (preserve original `tag`)
  4. If API call fails, `resolved_tag` remains `None` and log warning

**Task 5.1.6: Implement Hosted NIM Batch Enrichment**

Implement `enrich_hosted_nim_matches` function:
- Input: `&mut Vec<HostedNimMatch>`, `&NgcClient`
- Output: None (modify in place)
- Functionality:
  1. Collect all unique model_names
  2. Query NGC API for each model_name
  3. Populate function_id, status, container_image to corresponding Matches

---

## 6. Phase 4: Output and Integration

### 6.1 report.rs - Report Generation

**Task 6.1.1: Implement JSON Report Generation**

Implement `generate_json_report` function:
- Input: `ScanReport`, output file path
- Output: `Result<()>`
- Functionality:
  1. Use serde_json to serialize report
  2. Enable pretty print (2-space indent)
  3. Write to file

**Task 6.1.2: Implement CSV Report Generation**

Implement `generate_csv_reports` function:
- Input: `ScanReport`, output directory
- Output: `Result<()>`
- Functionality:
  1. Create **unified** `report.csv` file
  2. Write header: `source_type,nim_type,repository,file_path,line_number,image_url,tag,resolved_tag,endpoint_url,model_name,function_id,status,container_image,match_context`
  3. Write data from source_code.local_nim, source_code.hosted_nim, actions_workflow.local_nim, actions_workflow.hosted_nim in order
  4. Local NIM rows leave Hosted NIM fields (endpoint_url, etc.) empty
  5. Hosted NIM rows leave Local NIM fields (image_url, etc.) empty
  6. Handle special characters in fields (commas, quotes, newlines)

**Task 6.1.3: Implement Report Summary Calculation**

Implement `calculate_summary` function:
- Input: `NimFindings` (source_code), `NimFindings` (actions_workflow)
- Output: `Summary`
- Functionality:
  1. Calculate total Local NIM count (source_code + actions_workflow)
  2. Calculate total Hosted NIM count (source_code + actions_workflow)
  3. Calculate number of repos containing NIM (deduplicate repository field)
  4. Calculate CategorySummary for source_code (local_nim, hosted_nim counts)
  5. Calculate CategorySummary for actions_workflow (local_nim, hosted_nim counts)

### 6.2 main.rs - CLI Entry

**Task 6.2.1: Define Command Line Arguments**

Use clap derive macro to define `Args` struct:

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `config` | PathBuf | Yes | - | Configuration file path |
| `output` | PathBuf | No | `./output` | Output directory |
| `ngc_api_key` | String | **Yes** | - | NGC API Key (**required**, or use `NVIDIA_API_KEY` env var) |
| `github_token` | String | **Yes** | - | GitHub Token (**required**, or use `GITHUB_TOKEN` env var, for cloning private repos) |
| `workdir` | Option<PathBuf> | No | System temp dir | Work directory |
| `keep_repos` | bool | No | false | Keep cloned repositories |
| `verbose` | u8 | No | 0 | Log level (-v, -vv) |
| `jobs` | Option<usize> | No | CPU core count | Concurrency |

**Task 6.2.2: Implement Main Function Flow**

Implement `main` function execution flow:

```
1. Initialize Logging
   └── Set log level based on verbose parameter

2. Parse Command Line Arguments
   └── Use clap for parsing

3. Read Required Parameters
   └── NGC API Key: Prefer `--ngc-api-key`, then `NVIDIA_API_KEY` env var (required)
   └── GitHub Token: Prefer `--github-token`, then `GITHUB_TOKEN` env var (required)

4. Load Configuration
   └── Call config::load_config
   └── Call config::validate_config
   └── Call config::apply_defaults

5. Filter Enabled Repositories
   └── Only process repos with enabled = true

6. Create Work Directory
   └── Use specified workdir if provided
   └── Otherwise use tempfile to create temporary directory

7. Clone Repositories
   └── Call git_ops::clone_all_repos
   └── Record failed repositories

8. Scan Repositories
   └── Call scanner::scan_directory in parallel
   └── Collect all results

9. Call NGC API for Enrichment
   └── If API Key provided
   └── Call ngc_api::enrich_local_nim_matches (resolve latest tag)
   └── Call ngc_api::enrich_hosted_nim_matches (get Function details)

10. Generate Reports
    └── Call report::calculate_summary
    └── Build ScanReport
    └── Call report::generate_json_report
    └── Call report::generate_csv_reports

11. Cleanup
    └── If keep_repos = false
    └── Call git_ops::cleanup_repos

12. Output Summary
    └── Print scan result statistics
```

**Task 6.2.3: Implement Error Handling**

- Use `anyhow::Result` for unified error types
- Configuration-related errors cause program exit
- Single repository errors log warning but continue execution
- Final output includes list of failed repositories

---

## 7. Configuration File Example

### 7.1 repos.yaml

Create example configuration in `config/repos.yaml`:

```yaml
version: "1.0"

defaults:
  branch: main
  depth: 1

repos:
  - name: NVIDIA/GenerativeAIExamples
    url: https://github.com/NVIDIA/GenerativeAIExamples.git
    
  - name: NVIDIA/workbench-example-hybrid-rag
    url: https://github.com/NVIDIA/workbench-example-hybrid-rag.git
    
  - name: NVIDIA/nemo-guardrails
    url: https://github.com/NVIDIA/NeMo-Guardrails.git
    branch: develop
```

---

## 8. Testing Plan

### 8.1 Unit Tests

**config.rs Tests**:
- Test valid YAML configuration parsing
- Test invalid YAML format error handling
- Test default value merging logic

**scanner.rs Tests**:
- Test Local NIM regex matching various formats
- Test Hosted NIM regex matching various formats
- Test edge cases (URLs in comments, URLs in strings)
- Test file traversal correctly ignores directories

**ngc_api.rs Tests**:
- Test API response parsing
- Test caching functionality
- Test API error handling

**report.rs Tests**:
- Test JSON output format
- Test CSV special character escaping
- Test Summary calculation

### 8.2 Integration Tests

Create `tests/` directory containing:

1. **Test Repository Preparation**: Create test files with known NIM references
2. **End-to-End Testing**: Execute complete scan flow, verify output

### 8.3 Manual Testing

Prepare the following test case files:

**test_dockerfile**:
```dockerfile
FROM nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2:1.10.0
```

**test_compose.yaml**:
```yaml
services:
  nim:
    image: nvcr.io/nim/nvidia/llama:latest
```

**test_python.py**:
```python
client = OpenAI(base_url="https://ai.api.nvidia.com/v1")
response = client.chat.completions.create(model="nvidia/llama-3.1-nemotron")
```

**test_workflow.yml** (placed under `.github/workflows/` directory):
```yaml
name: Deploy
jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - name: Pull NIM
        run: docker pull nvcr.io/nim/nvidia/nemo-retriever:24.08
```

**Expected Result Verification**:
- `test_dockerfile`, `test_compose.yaml`, `test_python.py` results should appear under `source_code` category
- `test_workflow.yml` results should appear under `actions_workflow` category
- CSV file: `source_code_local_nim.csv` should contain Dockerfile and compose results
- CSV file: `actions_workflow_local_nim.csv` should contain workflow results

---

## 9. Implementation Checklist

### Phase 1 Checklist

- [ ] Cargo.toml configuration complete, all dependencies added
- [ ] models.rs all data structures defined (including SourceType enum)
- [ ] config.rs configuration loading, validation, default merging complete
- [ ] git_ops.rs clone, batch clone, cleanup functions complete
- [ ] Unit tests passing

### Phase 2 Checklist

- [ ] All regular expressions defined and tested
- [ ] File traversal function complete, correctly ignores directories
- [ ] Single file scanning function complete
- [ ] **Source type determination function complete (determine_source_type)**
- [ ] Local NIM extraction function complete
- [ ] Hosted NIM extraction function complete
- [ ] **Result classification function complete (categorize_results)**
- [ ] Deduplication function complete
- [ ] Unit tests passing

### Phase 3 Checklist

- [ ] NgcClient struct implemented (dual cache: local_nim_cache + hosted_nim_cache)
- [ ] Local NIM latest tag resolution function complete (calls NGC Container Registry API)
- [ ] Function search function complete (calls NVCF Functions List API)
- [ ] Function details retrieval function complete (calls NVCF Function Details API)
- [ ] Local NIM batch enrichment function complete (enrich_local_nim_matches)
- [ ] Hosted NIM batch enrichment function complete (enrich_hosted_nim_matches)
- [ ] API error handling complete (401/404/429/5xx)
- [ ] Unit tests passing

### Phase 4 Checklist

- [ ] JSON report generation function complete (top-level by source category)
- [ ] CSV report generation function complete (4 files: source_code_*, actions_workflow_*)
- [ ] Summary calculation function complete (includes category statistics)
- [ ] CLI argument parsing complete
- [ ] Main function flow complete
- [ ] Error handling complete
- [ ] Integration tests passing
- [ ] Manual testing passing

---

## 10. Important Notes

### 10.1 Regular Expression Writing

1. Escape special characters (`.` should be `\.`)
2. Use non-greedy matching to avoid over-matching
3. Consider case sensitivity (Dockerfile vs dockerfile)
4. Handle quote variants (single quotes, double quotes)

### 10.2 File Encoding

1. Assume all files are UTF-8 encoded
2. Skip files with illegal UTF-8 characters and log warning

### 10.3 Concurrency Safety

1. Use Arc to wrap shared data
2. Use Mutex to protect mutable state
3. NGC API client cache needs to be thread-safe

### 10.4 Performance Considerations

1. Precompile regular expressions
2. Use BufReader to read large files
3. Avoid unnecessary string copies
4. Control concurrency to avoid GitHub API rate limiting

### 10.5 Security Considerations

1. Don't output API Key in logs
2. Passing API Key via environment variables is more secure
3. Validate clone URL format to prevent command injection
