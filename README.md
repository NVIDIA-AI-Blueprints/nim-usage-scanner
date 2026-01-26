# NIM Usage Scanner

A static code analyzer that scans Git repositories to discover and catalog NVIDIA NIM (Inference Microservice) usage.

## Features

- **Multi-repo Scanning**: Clone and scan multiple repositories from a configuration file
- **Local NIM Detection**: Find `nvcr.io/nim/*` Docker image references
- **Hosted NIM Detection**: Find hosted endpoints and model references (publisher-whitelisted)
- **Source Classification**: Distinguish between source code and GitHub Actions workflow usage
- **NGC API Enrichment**: Resolve `latest` tags and fetch Function details
- **Query Mode**: Directly query NIM information by model/image name

## Quick Start

### Prerequisites

- Rust 1.70+ (for building from source)
- NVIDIA API Key (from [NGC](https://ngc.nvidia.com/), optional for enrichment)
- GitHub Token (for cloning private repositories, optional)

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
# Set environment variables (optional)
export NVIDIA_API_KEY="nvapi-xxx"
export GITHUB_TOKEN="ghp_xxx"

# Scan repositories defined in repos.yaml
./target/release/nim-usage-scanner scan -c config/repos.yaml

# Regenerate repos.yaml from Build Page before scanning
./target/release/nim-usage-scanner scan -c config/repos.yaml --refresh-repos

# Keep cloned repos and reuse on next run (auto pull latest)
./target/release/nim-usage-scanner scan -c config/repos.yaml --refresh-repos --workdir /tmp/blueprint-scan --keep-repos --jobs 4

# Output will be in ./output/report.json, ./output/report.csv, and ./output/report_aggregate.json
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

### Generate repos.yaml from Build Blueprints (optional)

You can generate `config/repos.yaml` directly from the Buil API
and each endpoint's spec ("View GitHub" link):

```bash
python scripts/generate_repos_from_ngc.py
```

Optional flags:
- `--label blueprint`
- `--page-size 1000`
- `--branch main`
- `--depth 1`
- `--output config/repos.yaml`

## Commands

### `scan` - Scan Repositories

```bash
nim-usage-scanner scan [OPTIONS] --config <CONFIG> [--ngc-api-key <KEY>] [--github-token <TOKEN>]
```

| Option | Description |
|--------|-------------|
| `-c, --config` | Path to repos.yaml (required) |
| `-o, --output` | Output directory (default: `./output`) |
| `--ngc-api-key` | NVIDIA API Key (or use `NVIDIA_API_KEY` env var, optional) |
| `--github-token` | GitHub Token (or use `GITHUB_TOKEN` env var, optional) |
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

## How Detection Works

### Local NIM (Docker Images)

Local NIMs are detected by scanning file contents for Docker image references:

- **Full image with tag**: `nvcr.io/nim/<namespace>/<name>:<tag>`
- **Image without tag**: `nvcr.io/nim/<namespace>/<name>` (tag defaults to `latest`)

Additional behavior:

- **YAML tag context**: In `.yaml`/`.yml`, if an image is found with `latest`, the scanner looks up to 3 lines ahead for a `tag:` field and uses it when present.
- **File types**: The scanner checks common source and config formats, including `yaml/yml`, `json`, `toml`, `env`, `Dockerfile`, `md`, and `ipynb`.

### Hosted NIM (API Endpoints + Model Names)

Hosted NIMs are detected by scanning for:

- **API endpoints** matching `https://{integrate|ai|build}.api.nvidia.com/...`
- **Model fields** such as `model = "org/name"` or `model: "org/name"`
- **Known client patterns** like `ChatNVIDIA(...)`, `NVIDIAEmbeddings(...)`, `NVIDIARerank(...)`
- **Build Page links** like `https://build.nvidia.com/org/model`

Model-name extraction:

- If a model name is not present on a line but an endpoint is, the scanner may try to extract `org/model` from the URL path.
- For YAML files, if an endpoint is found without a model name, the scanner searches up to 10 lines around it for a `model` or `model_name` field.

Publisher whitelist:

- The model prefix (`org` in `org/model`) must be in a **publisher whitelist** to be counted.
- The whitelist is fetched at runtime from the Build Page API and falls back to a built-in list if the API is unavailable.
- This whitelist applies to **all file types**, including `md` and `ipynb`.

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
