# NIM Usage Scanner Architecture Design Document

## 1. Project Overview

### 1.1 Project Goals

NIM Usage Scanner is a static code analysis tool implemented in Rust, designed to scan multiple Git repositories, detect NVIDIA NIM (Inference Microservice) usage, and generate structured reports.

### 1.2 Core Features

| Feature | Description |
|---------|-------------|
| Multi-repo Cloning | Batch clone target repositories based on configuration file |
| Local NIM Detection | Scan `nvcr.io/nim/*` Docker image references |
| Hosted NIM Detection | Scan `*.api.nvidia.com` API endpoints and model references |
| **Source Classification** | Distinguish between "Source Code References" and "Actions Workflow References" |
| NGC API Enrichment | Call NGC API to get detailed information (latest tag, Function details) |
| Report Generation | Output scan results in JSON and CSV formats, with statistics by source type |

### 1.3 Detection Source Classification

| Source Type | Path Pattern | Description |
|-------------|--------------|-------------|
| `source` | Files not under `.github/workflows/` | NIM references in regular source code and configuration files |
| `workflow` | `.github/workflows/*.yml` or `.github/workflows/*.yaml` | NIM references in GitHub Actions Workflows |

**Classification Purpose**:
- Source Code References: Represents NIMs that the project code depends on
- Workflow References: Represents NIMs used in CI/CD processes (possibly in testing or deployment stages)

### 1.4 Output Data Structure

**Top-level Structure**:

| Field | Description |
|-------|-------------|
| `scan_time` | Scan timestamp |
| `total_repos` | Total number of repositories scanned |
| `source_code` | NIM references in source code (non-workflow files) |
| `actions_workflow` | NIM references in Actions Workflows |
| `aggregated` | **Aggregated View**: Grouped by unique NIM, includes all occurrence locations |
| `summary` | Statistical summary |

**Aggregated View Fields** (`aggregated.local_nim[]` or `aggregated.hosted_nim[]`):

| Field | Description |
|-------|-------------|
| `image_url` / `model_name` | Unique NIM identifier |
| `locations[]` | List of all occurrence locations |
| `locations[].source_type` | `source_code` or `actions_workflow` |
| `locations[].repository` | Repository name |
| `locations[].file_path` | File path |
| `locations[].line_number` | Line number |
| `locations[].match_context` | Original matching line |

**Local NIM Fields** (`source_code.local_nim[]` or `actions_workflow.local_nim[]`):

| Field | Description | Example |
|-------|-------------|---------|
| `repository` | Repository name | `NVIDIA/GenerativeAIExamples` |
| `image_url` | Complete image URL | `nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2` |
| `tag` | Image version (original value) | `latest` |
| `resolved_tag` | Resolved version (via NGC API) | `1.10.0` (optional, only populated when original tag is latest) |
| `file_path` | Relative file path | `deploy/docker-compose.yaml` |
| `line_number` | Line number | `42` |
| `match_context` | Original matching line | `image: nvcr.io/nim/nvidia/llama:latest` |

**Hosted NIM Fields** (`source_code.hosted_nim[]` or `actions_workflow.hosted_nim[]`):

| Field | Description | Example |
|-------|-------------|---------|
| `repository` | Repository name | `NVIDIA/GenerativeAIExamples` |
| `endpoint_url` | API endpoint | `https://ai.api.nvidia.com/v1` |
| `model_name` | Model name | `nvidia/llama-3.1-nemotron-70b-instruct` |
| `file_path` | Relative file path | `src/llm_client.py` |
| `line_number` | Line number | `28` |
| `match_context` | Original matching line | `model="nvidia/llama-3.1-nemotron"` |
| `function_id` | NVCF Function ID | `b6429d64-38a0-4888-aac4-29c2d378d1c4` |
| `status` | Function status | `ACTIVE` |
| `container_image` | Underlying container image | `nvcr.io/nim/nvidia/llama:1.0.0` |

---

## 2. System Architecture

### 2.1 Overall Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           NIM Usage Scanner                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌──────────────┐                                                           │
│  │ repos.yaml   │  Configuration file: Defines list of repositories to scan │
│  └──────┬───────┘                                                           │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                        CLI Entry (main.rs)                           │   │
│  │  • Parse command line arguments                                      │   │
│  │  • Coordinate module execution                                       │   │
│  │  • Error handling and logging                                        │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                     Config Loader (config.rs)                         │   │
│  │  • Parse repos.yaml                                                   │   │
│  │  • Validate configuration                                             │   │
│  │  • Return repository list                                             │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                    Repo Cloner (git_ops.rs)                           │   │
│  │  • Batch clone repositories to temporary directory                    │   │
│  │  • Support branch specification                                       │   │
│  │  • Shallow clone optimization (--depth 1)                             │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                      Scanner (scanner.rs)                             │   │
│  │  ┌─────────────────────────┐  ┌─────────────────────────┐            │   │
│  │  │   Local NIM Scanner     │  │   Hosted NIM Scanner    │            │   │
│  │  │                         │  │                         │            │   │
│  │  │  • Dockerfile           │  │  • Python source        │            │   │
│  │  │  • docker-compose.yml   │  │  • JavaScript/TS        │            │   │
│  │  │  • Shell scripts        │  │  • YAML configuration   │            │   │
│  │  │  • YAML configuration   │  │  • Shell scripts        │            │   │
│  │  └─────────────────────────┘  └─────────────────────────┘            │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                    NGC API Client (ngc_api.rs)                        │   │
│  │  • Query Function ID by model name                                    │   │
│  │  • Get Function details (status, containerImage)                      │   │
│  │  • Cache API responses to avoid duplicate requests                    │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                  Report Generator (report.rs)                         │   │
│  │  • Merge scan results                                                 │   │
│  │  • Deduplication                                                      │   │
│  │  • Output JSON file                                                   │   │
│  │  • Output CSV file                                                    │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌───────────────┐  ┌───────────────┐                                      │
│  │ report.json   │  │ report.csv    │                                      │
│  └───────────────┘  └───────────────┘                                      │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Module Responsibilities

| Module | File | Responsibility |
|--------|------|----------------|
| CLI Entry | `main.rs` | Parse arguments, coordinate execution flow, error handling |
| Config Loader | `config.rs` | Parse YAML configuration file |
| Git Operations | `git_ops.rs` | Clone repositories, manage temporary directories |
| Scanner | `scanner.rs` | Execute file traversal and regex matching |
| NGC API | `ngc_api.rs` | Call NVIDIA NGC API |
| Report Generator | `report.rs` | Generate JSON/CSV output |
| Data Models | `models.rs` | Define data structures |

---

## 3. Data Flow Design

### 3.1 Execution Flow

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│  Read Config │ ──▶ │ Clone Repos  │ ──▶ │  Scan Files  │ ──▶ │  Call NGC   │
│  repos.yaml │     │  git clone  │     │  Regex Match │     │    API      │
└─────────────┘     └─────────────┘     └─────────────┘     └─────────────┘
                                                                   │
                                                                   ▼
┌─────────────┐     ┌─────────────┐     ┌─────────────────────────────────┐
│ report.csv  │ ◀── │ report.json │ ◀── │      Merge, Dedupe, Format       │
└─────────────┘     └─────────────┘     └─────────────────────────────────┘
```

### 3.2 Detailed Scan Flow

```
For each repository (parallel processing):
│
├── 1. Clone Repository
│       └── git clone --depth 1 <url> <temp_dir>
│
├── 2. Traverse Files
│       ├── Use ignore crate for traversal (auto-respects .gitignore)
│       ├── Filter by file extension
│       └── Skip node_modules, .git, vendor, etc.
│
├── 3. Line-by-line Scanning
│       ├── Local NIM Scan
│       │     ├── Match nvcr.io/nim/* pattern
│       │     └── Extract image URL and tag
│       │
│       └── Hosted NIM Scan
│             ├── Match *.api.nvidia.com pattern
│             ├── Match model="nvidia/*" pattern
│             └── Extract endpoint and model name
│
├── 4. Collect Results
│       └── Record file_path, line_number, match_context
│
└── 5. Cleanup Temporary Directory
```

### 3.3 NGC API Call Flow

```
For each detected Hosted NIM model:
│
├── 1. Check Cache
│       └── If already queried for this model, return cached result
│
├── 2. Query Function ID
│       ├── Call GET /v2/nvcf/functions to get function list
│       └── Fuzzy match Function by model name
│
├── 3. Get Function Version Details
│       ├── ⚠️ Call GET /v2/nvcf/functions/{id}/versions
│       ├── (Note: Do not directly access /functions/{id}, it returns 404)
│       ├── Take functions[0] (latest version)
│       └── Extract status, containerImage, models.name, etc.
│
└── 4. Cache Result
        └── Write to memory cache to avoid duplicate requests
```

### 3.4 Local NIM Latest Tag Resolution Flow

```
For each detected Local NIM (tag is latest or no tag):
│
├── 1. Parse Image Path
│       └── Extract namespace and model from nvcr.io/nim/nvidia/model-name:latest
│
├── 2. Call NGC Container Registry API
│       └── Query metadata for this image repository
│
├── 3. Extract Actual Version
│       └── Get the specific version corresponding to latestTag from response
│
└── 4. Update Result
        └── Update tag field from "latest" to actual version (e.g., "1.10.0")
```

---

## 4. NGC API Detailed Design

This section describes in detail all NGC API endpoints, request formats, and response parsing.

### 4.1 API Authentication

All NGC API calls require authentication information in the request header:

| Header | Value |
|--------|-------|
| `Authorization` | `Bearer <NGC_API_KEY>` |

**Environment Variable**: `NVIDIA_API_KEY`

### 4.2 Local NIM: Resolve Latest Tag

When detected Local NIM uses `latest` tag or no tag specified, this API is called to get the actual version number.

**API Endpoint**:

```
GET https://api.ngc.nvidia.com/v2/org/nim/team/{team}/repos/{model-name}
```

**Path Parameters**:

| Parameter | Description | Example |
|-----------|-------------|---------|
| `{team}` | Team/namespace | `nvidia` |
| `{model-name}` | Model name | `llama-3.2-nv-embedqa-1b-v2` |

**URL Construction Example**:

For image `nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2:latest`:
- Extract team = `nvidia`
- Extract model-name = `llama-3.2-nv-embedqa-1b-v2`
- API URL = `https://api.ngc.nvidia.com/v2/org/nim/team/nvidia/repos/llama-3.2-nv-embedqa-1b-v2`

**Response Fields (to extract)**:

| Field Path | Description | Purpose |
|------------|-------------|---------|
| `latestTag` | Latest version tag | Replace `latest` with actual version number |
| `latestVersionId` | Latest version ID | Optional to record |
| `description` | Model description | Optional to record |

**Response Example Structure**:

```json
{
  "name": "llama-3.2-nv-embedqa-1b-v2",
  "latestTag": "1.10.0",
  "latestVersionId": "v1.10.0",
  "description": "NVIDIA embedding model...",
  "...": "..."
}
```

### 4.3 Hosted NIM: Get Function Information

For detected Hosted NIM models, the following APIs are called to get detailed information.

> **Important Note**: Use the `/versions` endpoint to get Function details, not directly accessing `/functions/{id}`.

#### 4.3.1 Query Function List

**API Endpoint**:

```
GET https://api.nvcf.nvidia.com/v2/nvcf/functions
```

**Request Headers**:

| Header | Value |
|--------|-------|
| `Authorization` | `Bearer <NVIDIA_API_KEY>` |

**Query Parameters** (optional):

| Parameter | Description |
|-----------|-------------|
| `visibility` | Filter visibility (public/private) |

**Response Fields (to extract)**:

| Field Path | Description |
|------------|-------------|
| `functions[].id` | Function ID |
| `functions[].name` | Function name (used to match model name) |
| `functions[].status` | Function status |

**Matching Logic**:

Iterate through the `functions` array, find entries where the `name` field contains the target model name.

For example: Detected `model="nvidia/llama-3.1-nemotron-70b-instruct"`, search for Functions with `name` containing `llama-3.1-nemotron` in the response.

#### 4.3.2 Get Function Version Details (Correct Method)

> **⚠️ Note**: Directly accessing `/v2/nvcf/functions/{id}` may return 404. The correct way is to use the **`/versions`** endpoint.

**API Endpoint**:

```
GET https://api.nvcf.nvidia.com/v2/nvcf/functions/{function_id}/versions
```

**Request Headers**:

| Header | Value |
|--------|-------|
| `Authorization` | `Bearer <NVIDIA_API_KEY>` |

**Path Parameters**:

| Parameter | Description |
|-----------|-------------|
| `{function_id}` | Function ID obtained from 4.3.1 |

**Response Structure**:

```json
{
  "functions": [
    {
      "id": "b6429d64-38a0-4888-aac4-29c2d378d1c4",
      "name": "llama-3.1-nemotron-70b-instruct",
      "status": "ACTIVE",
      "containerImage": "nvcr.io/nim/nvidia/llama-3.1-nemotron-70b-instruct:1.0.0",
      "models": [
        {
          "name": "llama-3.1-nemotron-70b-instruct",
          "version": "1.0"
        }
      ],
      "...": "..."
    }
  ]
}
```

**Note**: The response is a version list (`functions` array), take the first element (latest version) to get details.

**Response Fields (to extract)**:

| Field Path | Output Field | Description |
|------------|--------------|-------------|
| `functions[0].id` | `function_id` | NVCF Function UUID |
| `functions[0].status` | `status` | Function status (ACTIVE/INACTIVE/DEPLOYING, etc.) |
| `functions[0].name` | `model_name` | Model name |
| `functions[0].containerImage` | `container_image` | Underlying container image address |
| `functions[0].models[0].name` | - | Can be used to verify model name |

#### 4.3.3 Alternative: Get Function ID via Endpoints API

If matching in 4.3.1 is unsuccessful, use the Endpoints API to directly get Function ID:

**API Endpoint**:

```
GET https://api.ngc.nvidia.com/v2/endpoints/{org}/{model-name}/spec
```

**Path Parameters**:

| Parameter | Description | Example |
|-----------|-------------|---------|
| `{org}` | Organization ID | `qc69jvmznzxy` (API Catalog Production org) |
| `{model-name}` | Model name (`.` converted to `_`) | `llama-3_2-nv-rerankqa-1b-v2` |

**Important**: The `.` (dot) in model name needs to be converted to `_` (underscore).

For example:
- Original model: `llama-3.2-nv-rerankqa-1b-v2`
- API path: `llama-3_2-nv-rerankqa-1b-v2`

**Response Example**:

```json
{
  "nvcfFunctionId": "b6429d64-38a0-4888-aac4-29c2d378d1c4",
  "...": "..."
}
```

Then use the obtained `nvcfFunctionId` to call 4.3.2 to get complete details.

### 4.4 API Call Flow Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           NGC API Call Flow                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                    Local NIM (latest tag resolution)                 │    │
│  │                                                                     │    │
│  │   Input: nvcr.io/nim/nvidia/model-name:latest                        │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   GET /v2/org/nim/team/nvidia/repos/model-name                      │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   Extract: latestTag = "1.10.0"                                     │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   Output: resolved_tag set to "1.10.0" (preserve original tag)       │    │
│  │                                                                     │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                    Hosted NIM (Function info retrieval)              │    │
│  │                                                                     │    │
│  │   Input: model = "nvidia/llama-3.1-nemotron-70b-instruct"            │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   Step 1: GET /v2/nvcf/functions                                    │    │
│  │           └── Iterate to find matching Function                      │    │
│  │           └── Get function_id                                        │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   Step 2: GET /v2/nvcf/functions/{function_id}/versions             │    │
│  │           └── ⚠️ Use /versions endpoint (not direct function access) │    │
│  │           └── Take functions[0] to get latest version details        │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   Output:                                                            │    │
│  │     • function_id = "b6429d64-..."                                  │    │
│  │     • status = "ACTIVE"                                             │    │
│  │     • container_image = "nvcr.io/nim/..."                           │    │
│  │     • name (from models[0].name or function name)                   │    │
│  │                                                                     │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │              Alternative: Endpoints API to get Function ID           │    │
│  │                                                                     │    │
│  │   Input: model = "llama-3.2-nv-rerankqa-1b-v2"                       │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   Convert: "." → "_" → "llama-3_2-nv-rerankqa-1b-v2"                │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   GET /v2/endpoints/{org}/{model-name}/spec                         │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   Extract: nvcfFunctionId → continue to call Step 2                  │    │
│  │                                                                     │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 4.5 Error Handling

| HTTP Status Code | Handling |
|------------------|----------|
| 200 | Success, parse response |
| 401 | Authentication failed, log error, skip API enrichment |
| 404 | Resource not found, leave field empty |
| 429 | Too many requests, wait and retry (max 3 times) |
| 5xx | Server error, log warning, leave field empty |

### 4.6 Caching Strategy

To avoid duplicate requests, implement the following cache:

| Cache Key | Cache Content | TTL |
|-----------|---------------|-----|
| `local:{team}/{model}` | latestTag version number | Program runtime |
| `hosted:{model_name}` | Function details | Program runtime |

---

## 5. Scan Rules Design

### 5.1 Local NIM Detection Rules

**Target Pattern**: `nvcr.io/nim/<namespace>/<name>:<tag>`

**Scan File Types**:

| File Type | Extension | Detection Pattern |
|-----------|-----------|-------------------|
| Dockerfile | `Dockerfile*` | `FROM nvcr.io/nim/...` |
| Docker Compose | `*.yml`, `*.yaml` | `image: nvcr.io/nim/...` |
| Shell Scripts | `*.sh`, `*.bash` | `docker pull nvcr.io/nim/...` |
| YAML Config | `*.yml`, `*.yaml` | Any `nvcr.io/nim/...` |

**Regular Expressions**:

| Pattern Name | Regular Expression | Description |
|--------------|-------------------|-------------|
| Full Image Reference | `nvcr\.io/nim/([a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+):([a-zA-Z0-9._-]+)` | Match image with tag |
| No Tag Reference | `nvcr\.io/nim/([a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+)(?:[^:a-zA-Z0-9._-])` | Match image without tag |

### 5.2 Hosted NIM Detection Rules

**Target Patterns**:
- API endpoint: `https://*.api.nvidia.com/*`
- Model name: `nvidia/*` or `meta/*`, etc.

**Scan File Types**:

| File Type | Extension | Detection Pattern |
|-----------|-----------|-------------------|
| Python | `*.py` | `base_url=`, `model=` |
| JavaScript/TS | `*.js`, `*.ts` | `baseURL:`, `model:` |
| YAML Config | `*.yml`, `*.yaml` | Any endpoint or model |
| Shell | `*.sh` | URLs in `curl` commands |

**Regular Expressions**:

| Pattern Name | Regular Expression | Description |
|--------------|-------------------|-------------|
| API Endpoint | `https://(?:integrate|ai|build)\.api\.nvidia\.com[^\s"'\)]*` | Match NVIDIA API URL |
| Model Parameter | `model\s*[=:]\s*["']([^"']+/[^"']+)["']` | Match model assignment |
| OpenAI Compatible Call | `ChatNVIDIA\s*\([^)]*model\s*=\s*["']([^"']+)["']` | Match LangChain NVIDIA call |

### 5.3 Exclusion Rules

**Excluded Directories**:

```
.git/
node_modules/
vendor/
__pycache__/
.venv/
venv/
target/
build/
dist/
```

**Excluded Files**:

```
*.lock
*.min.js
*.min.css
*.map
*.pyc
```

---

## 6. Configuration File Design

### 6.1 repos.yaml Format

```yaml
# repos.yaml - Configuration for repositories to scan
version: "1.0"

# Default configuration (can be overridden by individual repos)
defaults:
  branch: main
  depth: 1

# Repository list
repos:
  - name: NVIDIA/GenerativeAIExamples
    url: https://github.com/NVIDIA/GenerativeAIExamples.git
    branch: main
    
  - name: NVIDIA/workbench-example-hybrid-rag
    url: https://github.com/NVIDIA/workbench-example-hybrid-rag.git
    branch: main
    
  - name: NVIDIA/blueprint-streaming-rag
    url: https://github.com/NVIDIA/blueprint-streaming-rag.git
    # Uses branch from defaults
```

### 6.2 Configuration Field Description

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Repository identifier (used in reports) |
| `url` | string | Yes | Git clone URL |
| `branch` | string | No | Specified branch, defaults to main |
| `depth` | int | No | Clone depth, defaults to 1 |
| `enabled` | bool | No | Whether enabled, defaults to true |

---

## 7. Output Format Design

### 7.1 JSON Output Format

**Top level separated by source type, inner level separated by NIM type**:

```json
{
  "scan_time": "2025-01-21T10:30:00Z",
  "total_repos": 5,
  "source_code": {
    "local_nim": [
      {
        "repository": "NVIDIA/GenerativeAIExamples",
        "image_url": "nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2",
        "tag": "latest",
        "resolved_tag": "1.10.0",
        "file_path": "deploy/docker-compose.yaml",
        "line_number": 42,
        "match_context": "    image: nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2:latest"
      }
    ],
    "hosted_nim": [
      {
        "repository": "NVIDIA/GenerativeAIExamples",
        "endpoint_url": "https://ai.api.nvidia.com/v1",
        "model_name": "nvidia/llama-3.1-nemotron-70b-instruct",
        "file_path": "src/llm_client.py",
        "line_number": 28,
        "match_context": "    model=\"nvidia/llama-3.1-nemotron-70b-instruct\",",
        "function_id": "b6429d64-38a0-4888-aac4-29c2d378d1c4",
        "status": "ACTIVE",
        "container_image": "nvcr.io/nim/nvidia/llama-3.1-nemotron-70b-instruct:1.0.0"
      }
    ]
  },
  "actions_workflow": {
    "local_nim": [
      {
        "repository": "NVIDIA/GenerativeAIExamples",
        "image_url": "nvcr.io/nim/nvidia/nemo-retriever",
        "tag": "24.08",
        "file_path": ".github/workflows/deploy.yml",
        "line_number": 85,
        "match_context": "        image: nvcr.io/nim/nvidia/nemo-retriever:24.08"
      }
    ],
    "hosted_nim": []
  },
  "aggregated": {
    "local_nim": [
      {
        "image_url": "nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2",
        "tag": "latest",
        "resolved_tag": "1.10.0",
        "locations": [
          {
            "source_type": "source_code",
            "repository": "NVIDIA/GenerativeAIExamples",
            "file_path": "deploy/docker-compose.yaml",
            "line_number": 42,
            "match_context": "..."
          }
        ]
      }
    ],
    "hosted_nim": [
      {
        "model_name": "nvidia/llama-3.1-nemotron-70b-instruct",
        "endpoint_url": "https://ai.api.nvidia.com/v1",
        "function_id": "b6429d64-38a0-4888-aac4-29c2d378d1c4",
        "status": "ACTIVE",
        "container_image": "nvcr.io/nim/nvidia/llama-3.1-nemotron-70b-instruct:1.0.0",
        "locations": [
          {
            "source_type": "source_code",
            "repository": "NVIDIA/GenerativeAIExamples",
            "file_path": "src/llm_client.py",
            "line_number": 28,
            "match_context": "..."
          }
        ]
      }
    ]
  },
  "summary": {
    "total_local_nim": 12,
    "total_hosted_nim": 8,
    "repos_with_nim": 4,
    "source_code": {
      "local_nim": 8,
      "hosted_nim": 6
    },
    "actions_workflow": {
      "local_nim": 4,
      "hosted_nim": 2
    }
  }
}
```

### 7.2 CSV Output Format

Generate **1 unified CSV file** `report.csv`, containing all types of NIM references:

**Header**:
```csv
source_type,nim_type,repository,file_path,line_number,image_url,tag,resolved_tag,endpoint_url,model_name,function_id,status,container_image,match_context
```

**Field Description**:

| Field | Description |
|-------|-------------|
| `source_type` | `source_code` or `actions_workflow` |
| `nim_type` | `local_nim` or `hosted_nim` |
| `repository` | Repository name |
| `file_path` | File path |
| `line_number` | Line number |
| `image_url` | Local NIM image URL (empty for Hosted NIM) |
| `tag` | Local NIM original tag (empty for Hosted NIM) |
| `resolved_tag` | Local NIM resolved version (via NGC API) |
| `endpoint_url` | Hosted NIM API endpoint (empty for Local NIM) |
| `model_name` | Hosted NIM model name (empty for Local NIM) |
| `function_id` | Hosted NIM Function ID (via NGC API) |
| `status` | Hosted NIM status (via NGC API) |
| `container_image` | Hosted NIM underlying container image (via NGC API) |
| `match_context` | Original matching line |

**Example**:
```csv
source_type,nim_type,repository,file_path,line_number,image_url,tag,resolved_tag,endpoint_url,model_name,function_id,status,container_image,match_context
source_code,local_nim,NVIDIA/GenerativeAIExamples,deploy/docker-compose.yaml,42,nvcr.io/nim/nvidia/llama,latest,1.10.0,,,,,"image: nvcr.io/nim/..."
source_code,hosted_nim,NVIDIA/GenerativeAIExamples,src/llm_client.py,28,,,,https://ai.api.nvidia.com/v1,nvidia/llama,b6429d64-...,ACTIVE,nvcr.io/nim/...,"model=""nvidia/..."""
actions_workflow,local_nim,NVIDIA/GenerativeAIExamples,.github/workflows/deploy.yml,85,nvcr.io/nim/nvidia/nemo,24.08,,,,,,"image: nvcr.io/nim/..."
```

---

## 8. Error Handling Design

### 8.1 Error Types

| Error Type | Handling | Abort? |
|------------|----------|--------|
| Config file not found | Print error, exit | Yes |
| Config file format error | Print error, exit | Yes |
| Single repo clone failed | Log warning, continue with other repos | No |
| File read failed | Log warning, skip this file | No |
| NGC API call failed | Log warning, leave field empty | No |
| Output file write failed | Print error, exit | Yes |

### 8.2 Log Levels

| Level | Use Case |
|-------|----------|
| ERROR | Fatal error, program needs to exit |
| WARN | Non-fatal error, can continue execution |
| INFO | Normal execution information (repo processing progress, etc.) |
| DEBUG | Detailed debug information (regex match details, etc.) |

---

## 9. Performance Design

### 9.1 Parallel Processing

- Use `rayon` crate for repository-level parallel cloning
- Use `rayon` crate for file-level parallel scanning
- Control max concurrency to avoid resource exhaustion

### 9.2 Optimization Strategies

| Optimization | Implementation |
|--------------|----------------|
| Shallow Clone | `git clone --depth 1` |
| File Filtering | Filter by extension upfront |
| Directory Skipping | Skip node_modules and other large directories |
| Regex Precompilation | Compile regex at startup, reuse for matching |
| API Caching | Query NGC API only once for same model |

### 9.3 Resource Management

- Cloned repositories use temporary directories
- Automatically cleanup temporary directories after scanning
- Can preserve temporary directories via parameter for debugging

---

## 10. Command Line Interface Design

### 10.1 Subcommand Overview

The tool supports two main subcommands:

| Subcommand | Description |
|------------|-------------|
| `scan` | Scan multiple repositories, detect NIM usage |
| `query` | Query detailed information for a single NIM |

```bash
nim-usage-scanner <COMMAND> [OPTIONS]
```

### 10.2 scan Subcommand

Scan multiple repositories, detect Local NIM and Hosted NIM usage.

```bash
nim-usage-scanner scan --config <CONFIG_FILE> --ngc-api-key <KEY> --github-token <TOKEN> [OPTIONS]
```

| Parameter | Short | Long | Description |
|-----------|-------|------|-------------|
| Config File | `-c` | `--config` | repos.yaml path (**required**) |
| Output Directory | `-o` | `--output` | Output file directory, default `./output` |
| NGC API Key | | `--ngc-api-key` | NVIDIA API Key (**required**, or use `NVIDIA_API_KEY` env var) |
| GitHub Token | | `--github-token` | GitHub Token (**required**, or use `GITHUB_TOKEN` env var) |
| Work Directory | `-w` | `--workdir` | Temporary clone directory, default system temp |
| Keep Repos | | `--keep-repos` | Don't delete cloned repos after scanning |
| Log Level | `-v` | `--verbose` | Increase log verbosity (can stack -vv) |
| Concurrency | `-j` | `--jobs` | Max parallel tasks, default CPU core count |

### 10.3 query Subcommand

Query detailed information for a single NIM, supports Hosted NIM and Local NIM.

#### 10.3.1 query hosted-nim

Query Hosted NIM Function ID, status, containerImage, etc.

```bash
nim-usage-scanner query hosted-nim --model <MODEL_NAME> --ngc-api-key <KEY>
```

| Parameter | Short | Long | Description |
|-----------|-------|------|-------------|
| Model Name | `-m` | `--model` | Hosted NIM model name (e.g., `nvidia/llama-3.1-nemotron-70b-instruct`) |
| NGC API Key | | `--ngc-api-key` | NVIDIA API Key (**required**) |
| Log Level | `-v` | `--verbose` | Increase log verbosity |

**Return Information**:
- `functionId` - NVCF Function ID
- `status` - Function status (ACTIVE/INACTIVE/DEPLOYING)
- `containerImage` - Underlying container image
- `name` - Function name
- `inferenceUrl` - Inference API URL
- Other metadata

#### 10.3.2 query local-nim

Query Local NIM latest tag, description, etc.

```bash
nim-usage-scanner query local-nim --image <IMAGE_NAME> --ngc-api-key <KEY>
```

| Parameter | Short | Long | Description |
|-----------|-------|------|-------------|
| Image Name | `-i` | `--image` | Local NIM image name (e.g., `nvidia/llama-3.2-nv-embedqa-1b-v2`) |
| NGC API Key | | `--ngc-api-key` | NVIDIA API Key (**required**) |
| Log Level | `-v` | `--verbose` | Increase log verbosity |

**Return Information**:
- `latestTag` - Actual version number corresponding to latest (e.g., `1.10.0`)
- `description` - Image description
- `displayName` - Display name
- `publisher` - Publisher
- Other metadata

### 10.4 query Subcommand Feature Comparison

> ⚠️ **Important**: The queryable information differs between Hosted NIM and Local NIM, determined by their architecture.

| Information Type | Hosted NIM | Local NIM | Description |
|------------------|:----------:|:---------:|-------------|
| Function ID | ✅ | ❌ | Only Hosted NIM runs on NVCF |
| Status (ACTIVE/INACTIVE) | ✅ | ❌ | Hosted NIM is a managed service |
| Container Image | ✅ | ❌ | Underlying image of Hosted NIM |
| Latest Tag → Actual Version | ❌ | ✅ | Local NIM is a Docker image |
| Image Description | ❌ | ✅ | From NGC image registry |
| Inference URL | ✅ | ❌ | API endpoint for Hosted NIM |

### 10.5 Environment Variables

| Environment Variable | Description |
|---------------------|-------------|
| `NVIDIA_API_KEY` | NGC API Key (can substitute `--ngc-api-key` parameter) |
| `GITHUB_TOKEN` | GitHub Token (can substitute `--github-token` parameter, only needed for scan) |
| `RUST_LOG` | Log level (debug/info/warn/error) |

### 10.6 Usage Examples

```bash
# === scan subcommand ===

# Basic scan (provide parameters via environment variables)
export NVIDIA_API_KEY="nvapi-xxx"
export GITHUB_TOKEN="ghp_xxx"
nim-usage-scanner scan -c repos.yaml

# Specify output directory
nim-usage-scanner scan -c repos.yaml -o ./reports

# Verbose log mode
nim-usage-scanner scan -c repos.yaml -vv

# === query subcommand ===

# Query Hosted NIM information
nim-usage-scanner query hosted-nim \
  --model "nvidia/llama-3.1-nemotron-70b-instruct" \
  --ngc-api-key "nvapi-xxx"

# Query Local NIM information
nim-usage-scanner query local-nim \
  --image "nvidia/llama-3.2-nv-embedqa-1b-v2" \
  --ngc-api-key "nvapi-xxx"

# Query with full image path also works
nim-usage-scanner query local-nim \
  --image "nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2" \
  --ngc-api-key "nvapi-xxx"
```
