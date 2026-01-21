# NIM Usage Scanner

A static code analyzer that scans Git repositories to discover and catalog NVIDIA NIM (Inference Microservice) usage.

## Features

- **Multi-repo Scanning**: Clone and scan multiple repositories from a configuration file
- **Local NIM Detection**: Find `nvcr.io/nim/*` Docker image references
- **Hosted NIM Detection**: Find `*.api.nvidia.com` API endpoint and model references
- **Source Classification**: Distinguish between source code and GitHub Actions workflow usage
- **NGC API Enrichment**: Resolve `latest` tags and fetch Function details
- **Query Mode**: Directly query NIM information by model/image name

## Quick Start

### Prerequisites

- Rust 1.70+ (for building from source)
- NVIDIA API Key (from [NGC](https://ngc.nvidia.com/))
- GitHub Token (for cloning private repositories)

### Installation

```bash
# Build from source
cd nim-usage-scanner
cargo build --release

# Binary will be at ./target/release/nim-usage-scanner
```

### Basic Usage

#### 1. Scan Repositories

```bash
# Set environment variables
export NVIDIA_API_KEY="nvapi-xxx"
export GITHUB_TOKEN="ghp_xxx"

# Scan repositories defined in repos.yaml
./target/release/nim-usage-scanner scan -c config/repos.yaml

# Output will be in ./output/report.json and ./output/report.csv
```

#### 2. Query NIM Information

```bash
# Query Hosted NIM details
./target/release/nim-usage-scanner query hosted-nim \
  --model "nvidia/llama-3.1-nemotron-70b-instruct" \
  --ngc-api-key "nvapi-xxx"

# Query Local NIM details
./target/release/nim-usage-scanner query local-nim \
  --image "nvidia/llama-3.2-nv-embedqa-1b-v2" \
  --ngc-api-key "nvapi-xxx"
```

## Configuration

Create a `repos.yaml` file:

```yaml
version: "1.0"

defaults:
  branch: main
  depth: 1

repos:
  - name: NVIDIA/GenerativeAIExamples
    url: https://github.com/NVIDIA/GenerativeAIExamples.git
    
  - name: my-org/my-private-repo
    url: https://github.com/my-org/my-private-repo.git
    branch: develop
```

## Commands

### `scan` - Scan Repositories

```bash
nim-usage-scanner scan [OPTIONS] --config <CONFIG> --ngc-api-key <KEY> --github-token <TOKEN>
```

| Option | Description |
|--------|-------------|
| `-c, --config` | Path to repos.yaml (required) |
| `-o, --output` | Output directory (default: `./output`) |
| `--ngc-api-key` | NVIDIA API Key (or use `NVIDIA_API_KEY` env var) |
| `--github-token` | GitHub Token (or use `GITHUB_TOKEN` env var) |
| `-v, --verbose` | Increase logging verbosity |

### `query` - Query NIM Information

#### `query hosted-nim`

Query Hosted NIM (cloud-hosted inference service) information.

```bash
nim-usage-scanner query hosted-nim --model <MODEL> --ngc-api-key <KEY>
```

**Returns**: Function ID, status, containerImage, inference URL, etc.

#### `query local-nim`

Query Local NIM (Docker container) information.

```bash
nim-usage-scanner query local-nim --image <IMAGE> --ngc-api-key <KEY>
```

**Returns**: Latest tag (actual version), description, publisher, etc.

## ⚠️ Important Limitations

### Query Feature Differences

Hosted NIM and Local NIM are fundamentally different architectures, so the available information differs:

| Information | Hosted NIM | Local NIM | Reason |
|-------------|:----------:|:---------:|--------|
| **Function ID** | ✅ | ❌ | Only Hosted NIMs run on NVIDIA Cloud Functions (NVCF) |
| **Status** (ACTIVE/INACTIVE) | ✅ | ❌ | Hosted NIMs are managed cloud services |
| **Container Image** | ✅ | ❌ | Refers to the underlying container of Hosted NIM |
| **Latest Tag → Actual Version** | ❌ | ✅ | Local NIMs are Docker images with tags |
| **Description** | ❌ | ✅ | Comes from NGC Container Registry metadata |
| **Inference URL** | ✅ | ❌ | Hosted NIMs have cloud API endpoints |

### Why This Limitation Exists

- **Hosted NIM**: Runs on NVIDIA's cloud infrastructure (NVCF). Each Hosted NIM has a unique Function ID that tracks its deployment status, container image, and API endpoint.

- **Local NIM**: Is a Docker image that you pull and run locally. It has no "Function ID" or "status" because it's not a managed service - you manage it yourself.

### Practical Implications

```bash
# ✅ This works - get Hosted NIM function details
nim-usage-scanner query hosted-nim --model "nvidia/llama-3.1-nemotron-70b-instruct"
# Returns: functionId, status, containerImage, inferenceUrl...

# ✅ This works - get Local NIM image details
nim-usage-scanner query local-nim --image "nvidia/llama-3.2-nv-embedqa-1b-v2"
# Returns: latestTag, description, publisher...

# ❌ Cannot get "status" for Local NIM - it's not a managed service
# ❌ Cannot get "latestTag" for Hosted NIM - it's not a Docker image
```

## Output Formats

### JSON Report (`report.json`)

```json
{
  "scan_time": "2025-01-21T10:30:00Z",
  "total_repos": 5,
  "source_code": {
    "local_nim": [...],
    "hosted_nim": [...]
  },
  "actions_workflow": {
    "local_nim": [...],
    "hosted_nim": [...]
  },
  "aggregated": {
    "local_nim": [...],
    "hosted_nim": [...]
  },
  "summary": {...}
}
```

### CSV Report (`report.csv`)

Unified CSV with all findings:

```csv
source_type,nim_type,repository,file_path,line_number,image_url,tag,resolved_tag,endpoint_url,model_name,function_id,status,container_image,match_context
source_code,local_nim,NVIDIA/Example,Dockerfile,5,nvcr.io/nim/nvidia/llama,latest,1.10.0,,,,,"FROM nvcr.io/nim/..."
source_code,hosted_nim,NVIDIA/Example,src/main.py,42,,,,https://ai.api.nvidia.com,nvidia/llama,abc-123,ACTIVE,nvcr.io/...,"model=..."
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `NVIDIA_API_KEY` | NGC API Key (required) |
| `GITHUB_TOKEN` | GitHub Token (required for scan) |
| `RUST_LOG` | Log level: `debug`, `info`, `warn`, `error` |

## License

[Apache 2.0](LICENSE)
