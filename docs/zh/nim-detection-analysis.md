# NIM 检测方案分析

## 1. 检测目标

### 1.1 最终目标

我们需要获取以下信息：

| 类型 | 需要获取的信息 |
|------|---------------|
| **Local NIM** | NIM 镜像 URL、版本号（tag）、代码位置（文件+行号） |
| **Hosted NIM** | API endpoint URL、版本号、代码位置（文件+行号） |
| **Hosted NIM 补充信息** | Function ID、Status、Model Name、Container Image |

### 1.2 检测范围

| 检测场景 | 说明 |
|---------|------|
| 源代码中的引用 | 在源代码、Dockerfile、docker-compose 中找到 NIM 引用 |
| Actions 运行时使用 | 在 GitHub Actions 执行过程中实际使用的 NIM |

---

## 2. 检测方法

### 2.1 方法一：静态代码扫描

**原理**：通过正则表达式扫描源代码文件，提取 NIM 相关的 URL 和引用。

**扫描目标**：

| 文件类型 | 检测内容 | 示例 |
|---------|---------|------|
| Dockerfile | FROM 语句 | `FROM nvcr.io/nim/nvidia/llama:1.0.0` |
| docker-compose.yml | image 字段 | `image: nvcr.io/nim/nvidia/llama:1.0.0` |
| Python 源码 | API endpoint | `base_url="https://ai.api.nvidia.com/v1"` |
| Python 源码 | model 参数 | `model="nvidia/llama-3.1-nemotron"` |
| Shell 脚本 | docker pull 命令 | `docker pull nvcr.io/nim/...` |
| YAML 配置 | 硬编码 URL | `nim_endpoint: https://ai.api.nvidia.com` |

**检测正则示例**：

```python
# Local NIM 检测
LOCAL_NIM_PATTERNS = [
    r"nvcr\.io/nim/([^\s:]+):([^\s]+)",      # 带版本号
    r"nvcr\.io/nim/([^\s]+)",                 # 不带版本号
    r"FROM\s+nvcr\.io/nim/([^\s]+)",          # Dockerfile FROM
]

# Hosted NIM 检测
HOSTED_NIM_PATTERNS = [
    r"(https://integrate\.api\.nvidia\.com[^\s\"']*)",
    r"(https://ai\.api\.nvidia\.com[^\s\"']*)",
    r"model\s*=\s*[\"']([^\"']*nvidia[^\"']*)[\"']",
]
```

---

### 2.2 方法二：Actions 日志分析

**原理**：在 GitHub Actions 运行结束后，通过 GitHub API 获取 Job 日志，解析其中的 NIM 使用痕迹。

**分析内容**：

| 日志来源 | 检测内容 | 可靠性 |
|---------|---------|--------|
| docker pull 输出 | `Pulling from nvcr.io/nim/...` | ✅ 高 |
| docker run 输出 | 容器启动信息 | ✅ 高 |
| curl 命令输出 | API URL（如果有 -v 参数） | ⚠️ 中 |
| SDK debug 日志 | model 名称、请求详情 | ⚠️ 低 |
| 应用程序日志 | 自定义输出 | ⚠️ 依赖开发者 |

**检测正则示例**：

```python
# 从日志中提取 Local NIM
LOG_LOCAL_NIM_PATTERNS = [
    r"docker\s+pull\s+(nvcr\.io/nim/[^\s]+)",
    r"Pulling\s+from\s+nvcr\.io/nim/([^\s]+)",
    r"Digest:\s+sha256:([a-f0-9]+)",
]

# 从日志中提取 Hosted NIM
LOG_HOSTED_NIM_PATTERNS = [
    r"(https://[a-z.]*api\.nvidia\.com[^\s\"']+)",
    r"\"model\":\s*\"([^\"]+)\"",
]
```

---

## 3. 方法对比

### 3.1 静态代码扫描

| 维度 | 评价 |
|------|------|
| **准确性** | ⚠️ 中等 - 只能检测硬编码值，无法处理变量引用 |
| **覆盖范围** | ✅ 广 - 可扫描所有文件类型 |
| **行号信息** | ✅ 可获取 - 精确到行 |
| **版本信息** | ✅ 可获取 - 如果硬编码了版本 |
| **实时性** | ✅ 不需要运行 Actions |
| **依赖性** | ✅ 无外部依赖 |

**优点**：
1. 不需要运行 Actions，可在 PR 阶段检测
2. 可以获取精确的文件路径和行号
3. 不依赖开发者的日志输出习惯
4. 可以发现未使用但已定义的 NIM 引用

**缺点**：
1. 无法检测通过变量/环境变量引入的值
2. 无法检测动态拼接的 URL
3. 无法确认是否真正被执行
4. 可能产生误报（注释中的 URL、未使用的代码）

**无法检测的场景示例**：

```python
# 场景 1：环境变量
nim_url = os.environ.get("NIM_ENDPOINT")  # 无法知道具体值

# 场景 2：变量拼接
base = "nvcr.io"
path = "nim/nvidia/llama"
image = f"{base}/{path}:1.0.0"  # 静态扫描只能看到片段

# 场景 3：配置文件引用
with open("config.yaml") as f:
    config = yaml.load(f)
    model = config["nim"]["model"]  # 需要额外解析配置文件

# 场景 4：条件分支
if use_cloud:
    endpoint = "https://ai.api.nvidia.com"
else:
    endpoint = "http://localhost:8000"  # 不确定运行时用哪个
```

---

### 3.2 Actions 日志分析

| 维度 | 评价 |
|------|------|
| **准确性** | ⚠️ 依赖日志输出 - 只能检测日志中出现的信息 |
| **覆盖范围** | ⚠️ 有限 - 只能检测运行时执行的代码 |
| **行号信息** | ❌ 不可获取 - 日志中没有行号信息 |
| **版本信息** | ✅ 可获取 - docker pull 会输出完整信息 |
| **实时性** | ❌ 需要等待 Actions 运行完成 |
| **依赖性** | ⚠️ 依赖开发者的日志输出 |

**优点**：
1. 反映真实运行情况，减少误报
2. 可以检测动态生成的 URL（如果有输出）
3. docker pull 的输出是可靠的
4. 可以检测环境变量的实际值（如果有输出）

**缺点**：
1. 无法获取行号信息
2. 依赖开发者是否输出完整日志
3. 需要等待 Actions 运行完成
4. Hosted NIM 的 model 信息通常不会出现在日志中
5. 如果 Actions 失败，可能无法获取完整日志

**无法检测的场景示例**：

```python
# 场景 1：SDK 调用不输出详情
client = OpenAI(base_url="https://ai.api.nvidia.com/v1")
response = client.chat.completions.create(
    model="nvidia/llama-3.1-nemotron",  # 不会出现在日志中
    messages=[...]
)

# 场景 2：静默拉取
docker pull nvcr.io/nim/... > /dev/null 2>&1  # 输出被重定向

# 场景 3：HTTP 库静默请求
response = requests.post(url, json=data)  # 没有 verbose 日志

# 场景 4：环境变量在运行时使用但未打印
endpoint = os.environ["NIM_ENDPOINT"]  # 如果不 print，日志中没有
```

---

## 4. 检测场景覆盖分析

### 4.1 Local NIM 检测覆盖

| 场景 | 静态扫描 | 日志分析 | 覆盖情况 |
|------|---------|---------|---------|
| Dockerfile 中 `FROM nvcr.io/nim/...` | ✅ | ✅ | 完全覆盖 |
| docker-compose 中 `image: nvcr.io/nim/...` | ✅ | ✅ | 完全覆盖 |
| Shell 脚本中 `docker pull nvcr.io/nim/...` | ✅ | ✅ | 完全覆盖 |
| 变量拼接的镜像名 | ❌ | ✅ | 依赖日志 |
| 环境变量引用的镜像 | ❌ | ✅ | 依赖日志 |
| 输出被重定向的 docker pull | ✅ | ❌ | 依赖静态 |

### 4.2 Hosted NIM 检测覆盖

| 场景 | 静态扫描 | 日志分析 | 覆盖情况 |
|------|---------|---------|---------|
| 硬编码 `base_url="https://ai.api.nvidia.com"` | ✅ | ⚠️ | 静态更好 |
| 硬编码 `model="nvidia/llama"` | ✅ | ❌ | 只能静态 |
| 环境变量 `os.environ["NIM_MODEL"]` | ❌ | ⚠️ | 都困难 |
| curl 命令带 -v 参数 | ✅ | ✅ | 完全覆盖 |
| SDK 调用（无 debug 日志） | ✅ | ❌ | 只能静态 |

---

