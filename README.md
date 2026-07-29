# NIM Usage Scanner

A static code analyzer that scans Git repositories to discover and catalog NVIDIA NIM (Inference Microservice) usage.

## Features

- **Multi-repo Scanning**: Clone and scan multiple repositories from a configuration file
- **Local NIM Detection**: Find `nvcr.io/nim/*` Docker image references
- **Hosted NIM Detection**: Find hosted endpoints and model references (publisher-whitelisted)
- **Source Classification**: Distinguish between source code and GitHub Actions workflow usage
- **NGC API Enrichment**: Resolve `latest` tags and fetch Function details
- **Query Mode**: Directly query NIM information by model/image name
- **Deprecation Check**: Flag blueprints that reference a deprecated NIM (from `config/nims.deprecated.yaml`)

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

# Refresh repos.yaml from the Build catalog before scanning
./target/release/nim-usage-scanner scan -c config/repos.yaml --refresh-repos

# Also flag blueprints that reference a deprecated NIM (config/nims.deprecated.yaml)
./target/release/nim-usage-scanner scan -c config/repos.yaml --check-deprecation

# Use a persistent workdir and keep repos after scan (recommended for repeated runs)
# First run: clones into /tmp/blueprint-scan. Second and later runs: reuses existing dirs and pulls latest (no full clone).
./target/release/nim-usage-scanner scan -c config/repos.yaml --workdir /tmp/blueprint-scan --keep-repos --jobs 4
# If a repo fails to clone with a Git LFS "smudge filter" error, prefix GIT_LFS_SKIP_SMUDGE=1 (see Troubleshooting)
# Add --refresh-repos only when you want to refresh repos.yaml from the Build catalog before scanning

# Output will be in ./output/report.json, ./output/report.csv, and ./output/report_aggregate.json
# With --check-deprecation, ./output/deprecation_affected_blueprints.{json,csv} are added when any blueprint is affected
```

> **Note:** `--check-deprecation` shells out to `scripts/find_blueprints_affected_by_deprecation.py`, which requires **PyYAML**. The scanner runs it with the repo's `.venv` if present, otherwise `python3` on `PATH`. Set up the dependency once with:
>
> ```bash
> python3 -m venv .venv && .venv/bin/python -m pip install -r requirements.txt
> ```

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

`repos.yaml` groups repositories into three categories:

- **`repos_active`** — active on the Build catalog and not deprecated. **Scanned.** Full objects.
- **`repos_github_only`** — only on GitHub (not returned by the Build API). **Scanned.** Full objects.
- **`repos_deprecated`** — deprecated on Build. **Not scanned.** Names only.

```yaml
version: "1.0"

defaults:
  branch: main
  depth: 1

# Active on Build and not deprecated. Scanned.
repos_active:
  - name: NVIDIA-AI-Blueprints/rag
    url: https://github.com/NVIDIA-AI-Blueprints/rag.git
    branch: main
    enabled: true   # optional, defaults to true; set false to skip

# Only on GitHub (not returned by the Build API). Scanned.
repos_github_only:
  - name: NVIDIA/Mosaic
    url: https://github.com/NVIDIA/Mosaic
    branch: main
    enabled: true

# Deprecated on Build (DEPRECATION attribute). NOT scanned.
repos_deprecated:
  - NVIDIA-AI-Blueprints/llm-router
```

The scanner only ever reads this one file — it does no merging, ignoring, or
refreshing of its own. Keeping the lists in sync with the Build catalog is the
job of the refresh script below.

### Refresh repos.yaml from the Build catalog

`scripts/refresh_repos_config.py` reconciles `repos.yaml` with the live Build
catalog. It lists all blueprints via the **NGC catalog resources API**
(`/v2/search/catalog/resources/BLUEPRINT`), splits them into active vs deprecated
using each blueprint's **`DEPRECATION`** attribute, resolves each to its GitHub
repo via `/v2/blueprints/{orgName}/{name}/spec` ("View GitHub" link), then
updates `repos.yaml` and writes a `repos_refresh_summary.json` to describe the changes.

```bash
# Update in place
python scripts/refresh_repos_config.py --config config/repos.yaml --in-place

# Or write to a new file (leaves the input untouched)
python scripts/refresh_repos_config.py --config config/repos.yaml --output repos-refreshed.yaml
```

Behavior:
- New active blueprints are appended to `repos_active`; existing entries are kept
  verbatim (so `enabled: false` overrides survive).
- `repos_active` is grouped into **Enterprise Blueprints**, **Developer Examples**,
  **Partner Examples**, and **NemoClaw** sections, derived from each blueprint's
  `apicatalogtype_*` label (and repo owner for partners). The grouping is
  regenerated on every run, so it stays stable and comment-free of manual edits.
- Newly deprecated blueprints are added to `repos_deprecated`.
- `repos_github_only` is never touched (manually curated).
- Enabled active repos no longer returned by the catalog are reported as
  `removed_active_repos` but only deleted when you pass `--prune-active`.
  Disabled entries (`enabled: false`) are treated as inactive — they are never
  flagged as removed or pruned.

Optional flags: `--in-place`, `--output PATH`, `--summary-json PATH`,
`--prune-active`, `--org`, `--page-size`, `--branch`, `--depth`. Requires PyYAML
(`pip install -r requirements.txt`).

## Commands

### `scan` - Scan Repositories

```bash
nim-usage-scanner scan [OPTIONS] -c <CONFIG> [--ngc-api-key <KEY>] [--github-token <TOKEN>]
```

| Option | Description |
|--------|-------------|
| `-c, --config` | Path to repos.yaml (required) |
| `-o, --output` | Output directory (default: `./output`) |
| `-w, --workdir` | Working directory for cloning repos (optional; uses temp dir if omitted) |
| `--keep-repos` | Keep cloned repositories after scanning; with `--workdir`, next run reuses and pulls instead of cloning (default: false) |
| `-j, --jobs` | Maximum number of parallel jobs (optional) |
| `--refresh-repos` | Before scanning, run `scripts/refresh_repos_config.py --config <config> --in-place` to reconcile repos.yaml with the Build catalog (default: false) |
| `--check-deprecation` | After scanning, run `scripts/find_blueprints_affected_by_deprecation.py` to report blueprints referencing a deprecated NIM from `config/nims.deprecated.yaml`. Writes `deprecation_affected_blueprints.{json,csv}` to the output dir (only when at least one blueprint is affected). Requires PyYAML (see note in [Basic Usage](#basic-usage)) (default: false) |
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
- **File types**: The scanner checks common source and config formats: `py`, `yaml`/`yml`, `json`, `toml`, `env`, `Dockerfile` (or any filename starting with `Dockerfile`), `md`, `ipynb`, `sh`, `bash`, `js`, `ts`, `jsx`, `tsx`, `cfg`, `ini`, `conf`.

### Hosted NIM (API Endpoints + Model Names)

Hosted NIMs are detected by scanning for:

- **API endpoints** matching `https://{integrate|ai|build}.api.nvidia.com/...`
- **Model fields** such as `model = "org/name"`, `model: "org/name"`, or `model_name: "org/name"` (e.g. in YAML/docs)
- **Known client patterns** like `ChatNVIDIA(...)`, `NVIDIAEmbeddings(...)`, `NVIDIARerank(...)`
- **Environment or config assignments** such as `os.environ["APP_EMBEDDINGS_MODELNAME"] = "org/model"` (e.g. in notebooks)
- **Build Page links** like `https://build.nvidia.com/org/model`
- **Prose in docs** such as `for nvidia/llama-3.2-nv-embedqa-1b-v2 model` or typo `nvidia/llama-3.2-nv-embedqa-1b-v2model` (org must be in the runtime publisher whitelist)

For all of the above, the **org** in `org/model` can be any publisher name; only those in the **runtime publisher whitelist** (from the NGC filters API) are counted as Hosted NIM.

- In source/config files (e.g. .py, .yaml), if a model name is not present on a line but an endpoint URL is, the scanner may try to extract `org/model` from the URL path.
- For YAML files, if an endpoint is found without a model name, the scanner searches up to 10 lines around it for a `model` or `model_name` field.

Publisher whitelist:

- The model prefix (`org` in `org/model`) must be in a **publisher whitelist** to be counted.
- The whitelist is fetched at runtime from the **NGC catalog filters API** (`/v2/search/catalog/filters/ENDPOINT`), which is separate from the **catalog resources API** (`/v2/search/catalog/resources/BLUEPRINT`) used for listing blueprints (e.g. `--refresh-repos`). From the filters response we use only the **filterValue** field from the `filterCategory: "publisher"` entries. The API may return publishers such as nvidia, meta, mistralai, microsoft, google, qwen, deepseek_ai. If the API is unavailable or returns no publishers, a **built-in fallback** list is used (nvidia, meta, mistralai, google, deepseek, stg).
- **Matching is case-insensitive**: values are stored and compared in lowercase.
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

### Deprecation Report (`deprecation_affected_blueprints.{json,csv}`)

Produced only with `--check-deprecation`, and only when at least one blueprint is affected. Deprecated NIM identifiers are read from `config/nims.deprecated.yaml` (a flat `deprecated:` list) and matched **case-insensitively** as **substrings** against every NIM reference found by the scan (both hosted and local). Fields: `repository`, `repository_url`, `affected_hosted_nims`, `affected_local_nims`.

```json
[
  {
    "repository": "NVIDIA-AI-Blueprints/content-localization",
    "repository_url": "https://github.com/NVIDIA-AI-Blueprints/content-localization",
    "affected_hosted_nims": [],
    "affected_local_nims": ["nvcr.io/nim/nvidia/active-speaker-detection:1.0.0"]
  }
]
```

The deprecated list (`config/nims.deprecated.yaml`):

```yaml
version: "1.0"
deprecated:
  - nvidia/active-speaker-detection
  - nemotron-3-nano
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `NVIDIA_API_KEY` | NGC API Key (optional; used for tag resolution and query enrichment) |
| `GITHUB_TOKEN` | GitHub Token (optional; required only for cloning private repositories) |
| `RUST_LOG` | Log level: `debug`, `info`, `warn`, `error` |
| `GIT_LFS_SKIP_SMUDGE` | Set to `1` to skip Git LFS downloads when cloning. Needed for LFS-backed repos whose LFS objects your token can't access (see [Troubleshooting](#troubleshooting)); the scanner doesn't need LFS content. |

## Troubleshooting

### Clone fails with `smudge filter lfs failed`

**Symptom** — a repository fails to clone with an error like:

```
Smudge error: Error downloading images/hosting_options.png: batch response: Resource not accessible by personal access token
error: external filter 'git-lfs filter-process' failed
fatal: images/hosting_options.png: smudge filter lfs failed
warning: Clone succeeded, but checkout failed.
```

**Cause** — the repository stores binary assets (images, models, etc.) in **Git LFS**. The git
history clones fine, but during checkout `git` runs the LFS *smudge* filter, which calls GitHub's
LFS batch API using your `GITHUB_TOKEN`. GitHub rejects it with *"Resource not accessible by
personal access token"* when the token isn't authorized for that repo's LFS objects — a common
limitation of **fine-grained PATs** and SSO/SAML-gated organization repos, whose tokens can read
git objects but not LFS objects. The smudge filter's failure makes `git clone` exit non-zero, so
the scanner marks the repository as failed even though the source it needs was already downloaded.

**Fix** — the scanner only reads source text for NIM references and never needs LFS blob content,
so skip LFS downloads by setting `GIT_LFS_SKIP_SMUDGE=1` when scanning:

```bash
GIT_LFS_SKIP_SMUDGE=1 GITHUB_TOKEN="$GITHUB_TOKEN" \
  ./target/release/nim-usage-scanner scan -c config/repos.yaml \
  --workdir /tmp/blueprint-scan --keep-repos --jobs 4
```

LFS-tracked files are then left as small pointer text files, which is fine for scanning.

> **Note:** If a previous failed run left a partial `<org>_<repo>` directory in your `--workdir`,
> delete it first — otherwise the `--keep-repos` reuse path will try to update the broken checkout.

## License

[Apache 2.0](LICENSE)
