# NIM Detection Methodology Analysis

## 1. Detection Goals

### 1.1 Ultimate Goals

We need to obtain the following information:

| Type | Information to Obtain |
|------|----------------------|
| **Local NIM** | NIM image URL, version (tag), code location (file + line number) |
| **Hosted NIM** | API endpoint URL, version, code location (file + line number) |
| **Hosted NIM Supplementary Info** | Function ID, Status, Model Name, Container Image |

### 1.2 Detection Scope

| Detection Scenario | Description |
|-------------------|-------------|
| Source Code References | Find NIM references in source code, Dockerfile, docker-compose |
| Actions Runtime Usage | NIMs actually used during GitHub Actions execution |

---

## 2. Detection Methods

### 2.1 Method One: Static Code Scanning

**Principle**: Extract NIM-related URLs and references from source code files using regular expressions.

**Scan Targets**:

| File Type | Detection Content | Example |
|-----------|------------------|---------|
| Dockerfile | FROM statements | `FROM nvcr.io/nim/nvidia/llama:1.0.0` |
| docker-compose.yml | image field | `image: nvcr.io/nim/nvidia/llama:1.0.0` |
| Python Source | API endpoint | `base_url="https://ai.api.nvidia.com/v1"` |
| Python Source | model parameter | `model="nvidia/llama-3.1-nemotron"` |
| Shell Script | docker pull command | `docker pull nvcr.io/nim/...` |
| YAML Config | Hardcoded URLs | `nim_endpoint: https://ai.api.nvidia.com` |

**Detection Regex Examples**:

```python
# Local NIM Detection
LOCAL_NIM_PATTERNS = [
    r"nvcr\.io/nim/([^\s:]+):([^\s]+)",      # With version
    r"nvcr\.io/nim/([^\s]+)",                 # Without version
    r"FROM\s+nvcr\.io/nim/([^\s]+)",          # Dockerfile FROM
]

# Hosted NIM Detection
HOSTED_NIM_PATTERNS = [
    r"(https://integrate\.api\.nvidia\.com[^\s\"']*)",
    r"(https://ai\.api\.nvidia\.com[^\s\"']*)",
    r"model\s*=\s*[\"']([^\"']*nvidia[^\"']*)[\"']",
]
```

---

### 2.2 Method Two: Actions Log Analysis

**Principle**: After GitHub Actions run completes, obtain Job logs through GitHub API and parse NIM usage traces.

**Analysis Content**:

| Log Source | Detection Content | Reliability |
|------------|------------------|-------------|
| docker pull output | `Pulling from nvcr.io/nim/...` | ✅ High |
| docker run output | Container startup info | ✅ High |
| curl command output | API URL (if -v parameter used) | ⚠️ Medium |
| SDK debug logs | model name, request details | ⚠️ Low |
| Application logs | Custom output | ⚠️ Developer dependent |

**Detection Regex Examples**:

```python
# Extract Local NIM from logs
LOG_LOCAL_NIM_PATTERNS = [
    r"docker\s+pull\s+(nvcr\.io/nim/[^\s]+)",
    r"Pulling\s+from\s+nvcr\.io/nim/([^\s]+)",
    r"Digest:\s+sha256:([a-f0-9]+)",
]

# Extract Hosted NIM from logs
LOG_HOSTED_NIM_PATTERNS = [
    r"(https://[a-z.]*api\.nvidia\.com[^\s\"']+)",
    r"\"model\":\s*\"([^\"]+)\"",
]
```

---

## 3. Method Comparison

### 3.1 Static Code Scanning

| Dimension | Evaluation |
|-----------|------------|
| **Accuracy** | ⚠️ Medium - Can only detect hardcoded values, cannot handle variable references |
| **Coverage** | ✅ Wide - Can scan all file types |
| **Line Number Info** | ✅ Available - Precise to line |
| **Version Info** | ✅ Available - If version is hardcoded |
| **Real-time** | ✅ No need to run Actions |
| **Dependencies** | ✅ No external dependencies |

**Advantages**:
1. No need to run Actions, can detect at PR stage
2. Can obtain precise file paths and line numbers
3. Independent of developer's logging habits
4. Can discover NIM references that are defined but unused

**Disadvantages**:
1. Cannot detect values introduced via variables/environment variables
2. Cannot detect dynamically concatenated URLs
3. Cannot confirm if actually executed
4. May produce false positives (URLs in comments, unused code)

**Undetectable Scenario Examples**:

```python
# Scenario 1: Environment Variable
nim_url = os.environ.get("NIM_ENDPOINT")  # Cannot know the actual value

# Scenario 2: Variable Concatenation
base = "nvcr.io"
path = "nim/nvidia/llama"
image = f"{base}/{path}:1.0.0"  # Static scan only sees fragments

# Scenario 3: Config File Reference
with open("config.yaml") as f:
    config = yaml.load(f)
    model = config["nim"]["model"]  # Need additional config file parsing

# Scenario 4: Conditional Branch
if use_cloud:
    endpoint = "https://ai.api.nvidia.com"
else:
    endpoint = "http://localhost:8000"  # Uncertain which is used at runtime
```

---

### 3.2 Actions Log Analysis

| Dimension | Evaluation |
|-----------|------------|
| **Accuracy** | ⚠️ Depends on log output - Can only detect info appearing in logs |
| **Coverage** | ⚠️ Limited - Can only detect code executed at runtime |
| **Line Number Info** | ❌ Not Available - No line number info in logs |
| **Version Info** | ✅ Available - docker pull outputs complete info |
| **Real-time** | ❌ Need to wait for Actions to complete |
| **Dependencies** | ⚠️ Depends on developer's log output |

**Advantages**:
1. Reflects actual runtime situation, reduces false positives
2. Can detect dynamically generated URLs (if outputted)
3. docker pull output is reliable
4. Can detect actual values of environment variables (if outputted)

**Disadvantages**:
1. Cannot obtain line number info
2. Depends on whether developer outputs complete logs
3. Need to wait for Actions to complete
4. Hosted NIM model info usually doesn't appear in logs
5. If Actions fails, may not get complete logs

**Undetectable Scenario Examples**:

```python
# Scenario 1: SDK Call Doesn't Output Details
client = OpenAI(base_url="https://ai.api.nvidia.com/v1")
response = client.chat.completions.create(
    model="nvidia/llama-3.1-nemotron",  # Won't appear in logs
    messages=[...]
)

# Scenario 2: Silent Pull
docker pull nvcr.io/nim/... > /dev/null 2>&1  # Output redirected

# Scenario 3: HTTP Library Silent Request
response = requests.post(url, json=data)  # No verbose logging

# Scenario 4: Environment Variable Used at Runtime but Not Printed
endpoint = os.environ["NIM_ENDPOINT"]  # If not printed, not in logs
```

---

## 4. Detection Scenario Coverage Analysis

### 4.1 Local NIM Detection Coverage

| Scenario | Static Scan | Log Analysis | Coverage |
|----------|-------------|--------------|----------|
| `FROM nvcr.io/nim/...` in Dockerfile | ✅ | ✅ | Fully covered |
| `image: nvcr.io/nim/...` in docker-compose | ✅ | ✅ | Fully covered |
| `docker pull nvcr.io/nim/...` in Shell script | ✅ | ✅ | Fully covered |
| Variable-concatenated image name | ❌ | ✅ | Depends on logs |
| Environment variable-referenced image | ❌ | ✅ | Depends on logs |
| docker pull with redirected output | ✅ | ❌ | Depends on static |

### 4.2 Hosted NIM Detection Coverage

| Scenario | Static Scan | Log Analysis | Coverage |
|----------|-------------|--------------|----------|
| Hardcoded `base_url="https://ai.api.nvidia.com"` | ✅ | ⚠️ | Static is better |
| Hardcoded `model="nvidia/llama"` | ✅ | ❌ | Static only |
| Environment variable `os.environ["NIM_MODEL"]` | ❌ | ⚠️ | Both difficult |
| curl command with -v parameter | ✅ | ✅ | Fully covered |
| SDK call (no debug logs) | ✅ | ❌ | Static only |

---
