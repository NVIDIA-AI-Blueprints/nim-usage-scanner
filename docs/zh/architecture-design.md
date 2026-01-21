# NIM Usage Scanner 架构设计文档

## 1. 项目概述

### 1.1 项目目标

NIM Usage Scanner 是一个基于 Rust 实现的静态代码分析工具，用于扫描多个 Git 仓库，检测其中的 NVIDIA NIM（Inference Microservice）使用情况，并生成结构化报告。

### 1.2 核心功能

| 功能 | 说明 |
|------|------|
| 多仓库克隆 | 根据配置文件批量克隆目标仓库 |
| Local NIM 检测 | 扫描 `nvcr.io/nim/*` Docker 镜像引用 |
| Hosted NIM 检测 | 扫描 `*.api.nvidia.com` API endpoint 和 model 引用 |
| **来源分类** | 区分「源码引用」和「Actions Workflow 引用」 |
| NGC API 补充 | 调用 NGC API 获取详细信息（latest tag、Function 详情） |
| 报告生成 | 输出 JSON 和 CSV 格式的扫描结果，按来源分类统计 |

### 1.3 检测来源分类

| 来源类型 | 路径特征 | 说明 |
|---------|---------|------|
| `source` | 非 `.github/workflows/` 下的文件 | 普通源代码、配置文件中的 NIM 引用 |
| `workflow` | `.github/workflows/*.yml` 或 `.github/workflows/*.yaml` | GitHub Actions Workflow 中的 NIM 引用 |

**分类目的**：
- 源码引用：表示项目代码中依赖的 NIM
- Workflow 引用：表示 CI/CD 流程中使用的 NIM（可能是测试、部署阶段）

### 1.4 输出数据结构

**顶层结构**：

| 字段 | 说明 |
|------|------|
| `scan_time` | 扫描时间 |
| `total_repos` | 扫描的仓库总数 |
| `source_code` | 源代码中的 NIM 引用（非 workflow 文件） |
| `actions_workflow` | Actions Workflow 中的 NIM 引用 |
| `aggregated` | **聚合视图**：按唯一 NIM 分组，包含所有出现位置 |
| `summary` | 统计摘要 |

**聚合视图字段**（`aggregated.local_nim[]` 或 `aggregated.hosted_nim[]`）：

| 字段 | 说明 |
|------|------|
| `image_url` / `model_name` | NIM 唯一标识 |
| `locations[]` | 所有出现位置列表 |
| `locations[].source_type` | `source_code` 或 `actions_workflow` |
| `locations[].repository` | 仓库名称 |
| `locations[].file_path` | 文件路径 |
| `locations[].line_number` | 行号 |
| `locations[].match_context` | 匹配行原文 |

**Local NIM 字段**（`source_code.local_nim[]` 或 `actions_workflow.local_nim[]`）：

| 字段 | 说明 | 示例 |
|------|------|------|
| `repository` | 仓库名称 | `NVIDIA/GenerativeAIExamples` |
| `image_url` | 完整镜像 URL | `nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2` |
| `tag` | 镜像版本号（原始值） | `latest` |
| `resolved_tag` | 解析后的版本号（通过 NGC API） | `1.10.0`（可选，仅当原始 tag 为 latest 时填充） |
| `file_path` | 文件相对路径 | `deploy/docker-compose.yaml` |
| `line_number` | 行号 | `42` |
| `match_context` | 匹配行的原文 | `image: nvcr.io/nim/nvidia/llama:latest` |

**Hosted NIM 字段**（`source_code.hosted_nim[]` 或 `actions_workflow.hosted_nim[]`）：

| 字段 | 说明 | 示例 |
|------|------|------|
| `repository` | 仓库名称 | `NVIDIA/GenerativeAIExamples` |
| `endpoint_url` | API endpoint | `https://ai.api.nvidia.com/v1` |
| `model_name` | 模型名称 | `nvidia/llama-3.1-nemotron-70b-instruct` |
| `file_path` | 文件相对路径 | `src/llm_client.py` |
| `line_number` | 行号 | `28` |
| `match_context` | 匹配行的原文 | `model="nvidia/llama-3.1-nemotron"` |
| `function_id` | NVCF Function ID | `b6429d64-38a0-4888-aac4-29c2d378d1c4` |
| `status` | 函数状态 | `ACTIVE` |
| `container_image` | 底层容器镜像 | `nvcr.io/nim/nvidia/llama:1.0.0` |

---

## 2. 系统架构

### 2.1 整体架构图

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           NIM Usage Scanner                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌──────────────┐                                                           │
│  │ repos.yaml   │  配置文件：定义待扫描的仓库列表                              │
│  └──────┬───────┘                                                           │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                        CLI 入口 (main.rs)                             │   │
│  │  • 解析命令行参数                                                      │   │
│  │  • 协调各模块执行                                                      │   │
│  │  • 错误处理与日志                                                      │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                     Config Loader (config.rs)                         │   │
│  │  • 解析 repos.yaml                                                    │   │
│  │  • 验证配置有效性                                                      │   │
│  │  • 返回仓库列表                                                        │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                    Repo Cloner (git_ops.rs)                           │   │
│  │  • 批量克隆仓库到临时目录                                               │   │
│  │  • 支持指定分支                                                        │   │
│  │  • 浅克隆优化 (--depth 1)                                             │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                      Scanner (scanner.rs)                             │   │
│  │  ┌─────────────────────────┐  ┌─────────────────────────┐            │   │
│  │  │   Local NIM Scanner     │  │   Hosted NIM Scanner    │            │   │
│  │  │                         │  │                         │            │   │
│  │  │  • Dockerfile           │  │  • Python 源码          │            │   │
│  │  │  • docker-compose.yml   │  │  • JavaScript/TS        │            │   │
│  │  │  • Shell 脚本           │  │  • YAML 配置            │            │   │
│  │  │  • YAML 配置            │  │  • Shell 脚本           │            │   │
│  │  └─────────────────────────┘  └─────────────────────────┘            │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                    NGC API Client (ngc_api.rs)                        │   │
│  │  • 根据 model name 查询 Function ID                                   │   │
│  │  • 获取 Function 详情（status, containerImage）                        │   │
│  │  • 缓存 API 响应避免重复请求                                           │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                  Report Generator (report.rs)                         │   │
│  │  • 合并扫描结果                                                        │   │
│  │  • 去重处理                                                            │   │
│  │  • 输出 JSON 文件                                                      │   │
│  │  • 输出 CSV 文件                                                       │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│         │                                                                   │
│         ▼                                                                   │
│  ┌───────────────┐  ┌───────────────┐                                      │
│  │ report.json   │  │ report.csv    │                                      │
│  └───────────────┘  └───────────────┘                                      │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 模块职责

| 模块 | 文件 | 职责 |
|------|------|------|
| CLI 入口 | `main.rs` | 解析参数、协调执行流程、错误处理 |
| 配置加载 | `config.rs` | 解析 YAML 配置文件 |
| Git 操作 | `git_ops.rs` | 克隆仓库、管理临时目录 |
| 扫描器 | `scanner.rs` | 执行文件遍历和正则匹配 |
| NGC API | `ngc_api.rs` | 调用 NVIDIA NGC API |
| 报告生成 | `report.rs` | 生成 JSON/CSV 输出 |
| 数据模型 | `models.rs` | 定义数据结构 |

---

## 3. 数据流设计

### 3.1 执行流程

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   读取配置   │ ──▶ │   克隆仓库   │ ──▶ │   扫描文件   │ ──▶ │  调用 NGC   │
│  repos.yaml │     │  git clone  │     │  正则匹配   │     │    API      │
└─────────────┘     └─────────────┘     └─────────────┘     └─────────────┘
                                                                   │
                                                                   ▼
┌─────────────┐     ┌─────────────┐     ┌─────────────────────────────────┐
│ report.csv  │ ◀── │ report.json │ ◀── │       合并、去重、格式化         │
└─────────────┘     └─────────────┘     └─────────────────────────────────┘
```

### 3.2 扫描流程详解

```
对于每个仓库 (并行处理):
│
├── 1. 克隆仓库
│       └── git clone --depth 1 <url> <temp_dir>
│
├── 2. 遍历文件
│       ├── 使用 ignore crate 遍历（自动忽略 .gitignore 中的文件）
│       ├── 根据文件扩展名过滤
│       └── 跳过 node_modules, .git, vendor 等目录
│
├── 3. 逐行扫描
│       ├── Local NIM 扫描
│       │     ├── 匹配 nvcr.io/nim/* 模式
│       │     └── 提取 image URL 和 tag
│       │
│       └── Hosted NIM 扫描
│             ├── 匹配 *.api.nvidia.com 模式
│             ├── 匹配 model="nvidia/*" 模式
│             └── 提取 endpoint 和 model name
│
├── 4. 收集结果
│       └── 记录 file_path, line_number, match_context
│
└── 5. 清理临时目录
```

### 3.3 NGC API 调用流程

```
对于每个检测到的 Hosted NIM model:
│
├── 1. 检查缓存
│       └── 如果已查询过该 model，直接返回缓存结果
│
├── 2. 查询 Function ID
│       ├── 调用 GET /v2/nvcf/functions 获取函数列表
│       └── 根据 model name 模糊匹配 Function
│
├── 3. 获取 Function 版本详情
│       ├── ⚠️ 调用 GET /v2/nvcf/functions/{id}/versions
│       ├── （注意：不是直接访问 /functions/{id}，会返回 404）
│       ├── 取 functions[0]（最新版本）
│       └── 提取 status, containerImage, models.name 等字段
│
└── 4. 缓存结果
        └── 写入内存缓存，避免重复请求
```

### 3.4 Local NIM latest tag 解析流程

```
对于每个检测到的 Local NIM（tag 为 latest 或无 tag）:
│
├── 1. 解析镜像路径
│       └── 从 nvcr.io/nim/nvidia/model-name:latest 提取 namespace 和 model
│
├── 2. 调用 NGC Container Registry API
│       └── 查询该镜像仓库的元数据
│
├── 3. 提取实际版本号
│       └── 从响应中获取 latestTag 对应的具体版本
│
└── 4. 更新结果
        └── 将 tag 字段从 "latest" 更新为实际版本号（如 "1.10.0"）
```

---

## 4. NGC API 详细设计

本节详细描述所有需要调用的 NGC API 端点、请求格式和响应解析。

### 4.1 API 认证

所有 NGC API 调用需要在请求头中携带认证信息：

| Header | 值 |
|--------|-----|
| `Authorization` | `Bearer <NGC_API_KEY>` |

**环境变量**：`NVIDIA_API_KEY`

### 4.2 Local NIM：解析 latest tag

当检测到的 Local NIM 使用 `latest` tag 或未指定 tag 时，需要调用此 API 获取实际版本号。

**API 端点**：

```
GET https://api.ngc.nvidia.com/v2/org/nim/team/{team}/repos/{model-name}
```

**路径参数**：

| 参数 | 说明 | 示例 |
|------|------|------|
| `{team}` | 团队/命名空间 | `nvidia` |
| `{model-name}` | 模型名称 | `llama-3.2-nv-embedqa-1b-v2` |

**URL 构造示例**：

对于镜像 `nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2:latest`：
- 提取 team = `nvidia`
- 提取 model-name = `llama-3.2-nv-embedqa-1b-v2`
- API URL = `https://api.ngc.nvidia.com/v2/org/nim/team/nvidia/repos/llama-3.2-nv-embedqa-1b-v2`

**响应字段（需提取）**：

| 字段路径 | 说明 | 用途 |
|---------|------|------|
| `latestTag` | 最新版本标签 | 替换 `latest` 为实际版本号 |
| `latestVersionId` | 最新版本 ID | 可选记录 |
| `description` | 模型描述 | 可选记录 |

**响应示例结构**：

```json
{
  "name": "llama-3.2-nv-embedqa-1b-v2",
  "latestTag": "1.10.0",
  "latestVersionId": "v1.10.0",
  "description": "NVIDIA embedding model...",
  "...": "..."
}
```

### 4.3 Hosted NIM：获取 Function 信息

对于检测到的 Hosted NIM model，需要调用以下 API 获取详细信息。

> **重要说明**：使用 `/versions` 端点获取 Function 详情，而非直接访问 `/functions/{id}`。

#### 4.3.1 查询 Function 列表

**API 端点**：

```
GET https://api.nvcf.nvidia.com/v2/nvcf/functions
```

**请求头**：

| Header | 值 |
|--------|-----|
| `Authorization` | `Bearer <NVIDIA_API_KEY>` |

**查询参数**（可选）：

| 参数 | 说明 |
|------|------|
| `visibility` | 过滤可见性（public/private） |

**响应字段（需提取）**：

| 字段路径 | 说明 |
|---------|------|
| `functions[].id` | Function ID |
| `functions[].name` | Function 名称（用于匹配 model name） |
| `functions[].status` | 函数状态 |

**匹配逻辑**：

遍历 `functions` 数组，查找 `name` 字段包含目标 model name 的条目。

例如：检测到 `model="nvidia/llama-3.1-nemotron-70b-instruct"`，在响应中查找 `name` 包含 `llama-3.1-nemotron` 的 Function。

#### 4.3.2 获取 Function 版本详情（正确方式）

> **⚠️ 注意**：直接访问 `/v2/nvcf/functions/{id}` 可能返回 404。正确的方式是使用 **`/versions`** 端点。

**API 端点**：

```
GET https://api.nvcf.nvidia.com/v2/nvcf/functions/{function_id}/versions
```

**请求头**：

| Header | 值 |
|--------|-----|
| `Authorization` | `Bearer <NVIDIA_API_KEY>` |

**路径参数**：

| 参数 | 说明 |
|------|------|
| `{function_id}` | 从 4.3.1 获取的 Function ID |

**响应结构**：

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

**注意**：响应是一个版本列表（`functions` 数组），取第一个元素（最新版本）获取详情。

**响应字段（需提取）**：

| 字段路径 | 对应输出字段 | 说明 |
|---------|-------------|------|
| `functions[0].id` | `function_id` | NVCF Function UUID |
| `functions[0].status` | `status` | 函数状态（ACTIVE/INACTIVE/DEPLOYING 等） |
| `functions[0].name` | `model_name` | 模型名称 |
| `functions[0].containerImage` | `container_image` | 底层容器镜像地址 |
| `functions[0].models[0].name` | - | 可用于验证 model name |

#### 4.3.3 备用方案：通过 Endpoints API 获取 Function ID

如果 4.3.1 的匹配不成功，可使用 Endpoints API 直接获取 Function ID：

**API 端点**：

```
GET https://api.ngc.nvidia.com/v2/endpoints/{org}/{model-name}/spec
```

**路径参数**：

| 参数 | 说明 | 示例 |
|------|------|------|
| `{org}` | 组织 ID | `qc69jvmznzxy`（API Catalog Production org） |
| `{model-name}` | 模型名称（`.` 转换为 `_`） | `llama-3_2-nv-rerankqa-1b-v2` |

**重要**：model name 中的 `.`（点）需要转换为 `_`（下划线）。

例如：
- 原始 model：`llama-3.2-nv-rerankqa-1b-v2`
- API 路径：`llama-3_2-nv-rerankqa-1b-v2`

**响应示例**：

```json
{
  "nvcfFunctionId": "b6429d64-38a0-4888-aac4-29c2d378d1c4",
  "...": "..."
}
```

然后使用获取到的 `nvcfFunctionId` 调用 4.3.2 获取完整详情。

### 4.4 API 调用流程图

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           NGC API 调用流程                                   │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                    Local NIM (latest tag 解析)                       │    │
│  │                                                                     │    │
│  │   输入: nvcr.io/nim/nvidia/model-name:latest                         │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   GET /v2/org/nim/team/nvidia/repos/model-name                      │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   提取: latestTag = "1.10.0"                                        │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   输出: resolved_tag 设置为 "1.10.0"（保留原始 tag）                   │    │
│  │                                                                     │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                    Hosted NIM (Function 信息获取)                    │    │
│  │                                                                     │    │
│  │   输入: model = "nvidia/llama-3.1-nemotron-70b-instruct"             │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   Step 1: GET /v2/nvcf/functions                                    │    │
│  │           └── 遍历查找匹配的 Function                                 │    │
│  │           └── 获取 function_id                                       │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   Step 2: GET /v2/nvcf/functions/{function_id}/versions             │    │
│  │           └── ⚠️ 使用 /versions 端点（非直接访问 function）            │    │
│  │           └── 取 functions[0] 获取最新版本详情                        │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   输出:                                                              │    │
│  │     • function_id = "b6429d64-..."                                  │    │
│  │     • status = "ACTIVE"                                             │    │
│  │     • container_image = "nvcr.io/nim/..."                           │    │
│  │     • name (from models[0].name 或 function name)                   │    │
│  │                                                                     │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │              备用方案: Endpoints API 获取 Function ID                 │    │
│  │                                                                     │    │
│  │   输入: model = "llama-3.2-nv-rerankqa-1b-v2"                        │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   转换: "." → "_" → "llama-3_2-nv-rerankqa-1b-v2"                   │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   GET /v2/endpoints/{org}/{model-name}/spec                         │    │
│  │                         │                                           │    │
│  │                         ▼                                           │    │
│  │   提取: nvcfFunctionId → 继续调用 Step 2                             │    │
│  │                                                                     │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 4.5 错误处理

| HTTP 状态码 | 处理方式 |
|------------|---------|
| 200 | 成功，解析响应 |
| 401 | 认证失败，记录错误，跳过 API 补充 |
| 404 | 资源不存在，该字段留空 |
| 429 | 请求过多，等待后重试（最多 3 次） |
| 5xx | 服务端错误，记录警告，该字段留空 |

### 4.6 缓存策略

为避免重复请求，实现以下缓存：

| 缓存 Key | 缓存内容 | TTL |
|---------|---------|-----|
| `local:{team}/{model}` | latestTag 版本号 | 程序运行期间 |
| `hosted:{model_name}` | Function 详情 | 程序运行期间 |

---

## 5. 扫描规则设计

### 4.1 Local NIM 检测规则

**目标模式**：`nvcr.io/nim/<namespace>/<name>:<tag>`

**扫描文件类型**：

| 文件类型 | 扩展名 | 检测模式 |
|---------|--------|---------|
| Dockerfile | `Dockerfile*` | `FROM nvcr.io/nim/...` |
| Docker Compose | `*.yml`, `*.yaml` | `image: nvcr.io/nim/...` |
| Shell 脚本 | `*.sh`, `*.bash` | `docker pull nvcr.io/nim/...` |
| YAML 配置 | `*.yml`, `*.yaml` | 任意 `nvcr.io/nim/...` |

**正则表达式**：

| 模式名称 | 正则表达式 | 说明 |
|---------|-----------|------|
| 完整镜像引用 | `nvcr\.io/nim/([a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+):([a-zA-Z0-9._-]+)` | 匹配带 tag 的镜像 |
| 无 tag 引用 | `nvcr\.io/nim/([a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+)(?:[^:a-zA-Z0-9._-])` | 匹配不带 tag 的镜像 |

### 4.2 Hosted NIM 检测规则

**目标模式**：
- API endpoint: `https://*.api.nvidia.com/*`
- Model name: `nvidia/*` 或 `meta/*` 等

**扫描文件类型**：

| 文件类型 | 扩展名 | 检测模式 |
|---------|--------|---------|
| Python | `*.py` | `base_url=`, `model=` |
| JavaScript/TS | `*.js`, `*.ts` | `baseURL:`, `model:` |
| YAML 配置 | `*.yml`, `*.yaml` | 任意 endpoint 或 model |
| Shell | `*.sh` | `curl` 命令中的 URL |

**正则表达式**：

| 模式名称 | 正则表达式 | 说明 |
|---------|-----------|------|
| API Endpoint | `https://(?:integrate|ai|build)\.api\.nvidia\.com[^\s"'\)]*` | 匹配 NVIDIA API URL |
| Model 参数 | `model\s*[=:]\s*["']([^"']+/[^"']+)["']` | 匹配 model 赋值 |
| OpenAI 兼容调用 | `ChatNVIDIA\s*\([^)]*model\s*=\s*["']([^"']+)["']` | 匹配 LangChain NVIDIA 调用 |

### 4.3 排除规则

**排除目录**：

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

**排除文件**：

```
*.lock
*.min.js
*.min.css
*.map
*.pyc
```

---

## 6. 配置文件设计

### 6.1 repos.yaml 格式

```yaml
# repos.yaml - 待扫描仓库配置
version: "1.0"

# 默认配置（可被单个仓库覆盖）
defaults:
  branch: main
  depth: 1

# 仓库列表
repos:
  - name: NVIDIA/GenerativeAIExamples
    url: https://github.com/NVIDIA/GenerativeAIExamples.git
    branch: main
    
  - name: NVIDIA/workbench-example-hybrid-rag
    url: https://github.com/NVIDIA/workbench-example-hybrid-rag.git
    branch: main
    
  - name: NVIDIA/blueprint-streaming-rag
    url: https://github.com/NVIDIA/blueprint-streaming-rag.git
    # 使用 defaults 中的 branch
```

### 6.2 配置字段说明

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | string | 是 | 仓库标识名（用于报告） |
| `url` | string | 是 | Git clone URL |
| `branch` | string | 否 | 指定分支，默认 main |
| `depth` | int | 否 | 克隆深度，默认 1 |
| `enabled` | bool | 否 | 是否启用，默认 true |

---

## 7. 输出格式设计

### 7.1 JSON 输出格式

**最外层按来源类型区分，内层按 NIM 类型区分**：

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

### 7.2 CSV 输出格式

生成 **1 个统一的 CSV 文件** `report.csv`，包含所有类型的 NIM 引用：

**表头**：
```csv
source_type,nim_type,repository,file_path,line_number,image_url,tag,resolved_tag,endpoint_url,model_name,function_id,status,container_image,match_context
```

**字段说明**：

| 字段 | 说明 |
|------|------|
| `source_type` | `source_code` 或 `actions_workflow` |
| `nim_type` | `local_nim` 或 `hosted_nim` |
| `repository` | 仓库名称 |
| `file_path` | 文件路径 |
| `line_number` | 行号 |
| `image_url` | Local NIM 镜像 URL（Hosted NIM 为空） |
| `tag` | Local NIM 原始 tag（Hosted NIM 为空） |
| `resolved_tag` | Local NIM 解析后的版本号（通过 NGC API） |
| `endpoint_url` | Hosted NIM API 端点（Local NIM 为空） |
| `model_name` | Hosted NIM 模型名称（Local NIM 为空） |
| `function_id` | Hosted NIM Function ID（通过 NGC API） |
| `status` | Hosted NIM 状态（通过 NGC API） |
| `container_image` | Hosted NIM 底层容器镜像（通过 NGC API） |
| `match_context` | 匹配行原文 |

**示例**：
```csv
source_type,nim_type,repository,file_path,line_number,image_url,tag,resolved_tag,endpoint_url,model_name,function_id,status,container_image,match_context
source_code,local_nim,NVIDIA/GenerativeAIExamples,deploy/docker-compose.yaml,42,nvcr.io/nim/nvidia/llama,latest,1.10.0,,,,,"image: nvcr.io/nim/..."
source_code,hosted_nim,NVIDIA/GenerativeAIExamples,src/llm_client.py,28,,,,https://ai.api.nvidia.com/v1,nvidia/llama,b6429d64-...,ACTIVE,nvcr.io/nim/...,"model=""nvidia/..."""
actions_workflow,local_nim,NVIDIA/GenerativeAIExamples,.github/workflows/deploy.yml,85,nvcr.io/nim/nvidia/nemo,24.08,,,,,,"image: nvcr.io/nim/..."
```

---

## 8. 错误处理设计

### 8.1 错误类型

| 错误类型 | 处理方式 | 是否中断 |
|---------|---------|---------|
| 配置文件不存在 | 打印错误，退出 | 是 |
| 配置文件格式错误 | 打印错误，退出 | 是 |
| 单个仓库克隆失败 | 记录警告，继续处理其他仓库 | 否 |
| 文件读取失败 | 记录警告，跳过该文件 | 否 |
| NGC API 调用失败 | 记录警告，该字段留空 | 否 |
| 输出文件写入失败 | 打印错误，退出 | 是 |

### 8.2 日志级别

| 级别 | 使用场景 |
|------|---------|
| ERROR | 致命错误，程序需要退出 |
| WARN | 非致命错误，可继续执行 |
| INFO | 正常执行信息（仓库处理进度等） |
| DEBUG | 详细调试信息（正则匹配详情等） |

---

## 9. 性能设计

### 9.1 并行处理

- 使用 `rayon` crate 实现仓库级别并行克隆
- 使用 `rayon` crate 实现文件级别并行扫描
- 控制最大并发数，避免资源耗尽

### 9.2 优化策略

| 优化点 | 实现方式 |
|-------|---------|
| 浅克隆 | `git clone --depth 1` |
| 文件过滤 | 根据扩展名提前过滤 |
| 目录跳过 | 跳过 node_modules 等大目录 |
| 正则预编译 | 启动时编译正则，复用匹配 |
| API 缓存 | 相同 model 只查询一次 NGC API |

### 9.3 资源管理

- 克隆的仓库使用临时目录
- 扫描完成后自动清理临时目录
- 可通过参数保留临时目录用于调试

---

## 10. 命令行接口设计

### 10.1 子命令概览

工具支持两个主要子命令：

| 子命令 | 说明 |
|--------|------|
| `scan` | 扫描多个仓库，检测 NIM 使用情况 |
| `query` | 查询单个 NIM 的详细信息 |

```bash
nim-usage-scanner <COMMAND> [OPTIONS]
```

### 10.2 scan 子命令

扫描多个仓库，检测 Local NIM 和 Hosted NIM 使用情况。

```bash
nim-usage-scanner scan --config <CONFIG_FILE> --ngc-api-key <KEY> --github-token <TOKEN> [OPTIONS]
```

| 参数 | 短选项 | 长选项 | 说明 |
|------|--------|--------|------|
| 配置文件 | `-c` | `--config` | repos.yaml 路径（**必填**） |
| 输出目录 | `-o` | `--output` | 输出文件目录，默认 `./output` |
| NGC API Key | | `--ngc-api-key` | NVIDIA API Key（**必填**，或使用 `NVIDIA_API_KEY` 环境变量） |
| GitHub Token | | `--github-token` | GitHub Token（**必填**，或使用 `GITHUB_TOKEN` 环境变量） |
| 工作目录 | `-w` | `--workdir` | 临时克隆目录，默认系统临时目录 |
| 保留目录 | | `--keep-repos` | 扫描后不删除克隆的仓库 |
| 日志级别 | `-v` | `--verbose` | 增加日志详细程度（可叠加 -vv） |
| 并发数 | `-j` | `--jobs` | 最大并行任务数，默认 CPU 核心数 |

### 10.3 query 子命令

查询单个 NIM 的详细信息，支持 Hosted NIM 和 Local NIM。

#### 10.3.1 query hosted-nim

查询 Hosted NIM 的 Function ID、status、containerImage 等信息。

```bash
nim-usage-scanner query hosted-nim --model <MODEL_NAME> --ngc-api-key <KEY>
```

| 参数 | 短选项 | 长选项 | 说明 |
|------|--------|--------|------|
| 模型名称 | `-m` | `--model` | Hosted NIM 模型名（如 `nvidia/llama-3.1-nemotron-70b-instruct`） |
| NGC API Key | | `--ngc-api-key` | NVIDIA API Key（**必填**） |
| 日志级别 | `-v` | `--verbose` | 增加日志详细程度 |

**返回信息**：
- `functionId` - NVCF Function ID
- `status` - 函数状态（ACTIVE/INACTIVE/DEPLOYING）
- `containerImage` - 底层容器镜像
- `name` - 函数名称
- `inferenceUrl` - 推理 API URL
- 其他元数据

#### 10.3.2 query local-nim

查询 Local NIM 的 latest tag、描述等信息。

```bash
nim-usage-scanner query local-nim --image <IMAGE_NAME> --ngc-api-key <KEY>
```

| 参数 | 短选项 | 长选项 | 说明 |
|------|--------|--------|------|
| 镜像名称 | `-i` | `--image` | Local NIM 镜像名（如 `nvidia/llama-3.2-nv-embedqa-1b-v2`） |
| NGC API Key | | `--ngc-api-key` | NVIDIA API Key（**必填**） |
| 日志级别 | `-v` | `--verbose` | 增加日志详细程度 |

**返回信息**：
- `latestTag` - latest 对应的实际版本号（如 `1.10.0`）
- `description` - 镜像描述
- `displayName` - 显示名称
- `publisher` - 发布者
- 其他元数据

### 10.4 query 子命令功能对比

> ⚠️ **重要**：Hosted NIM 和 Local NIM 的可查询信息不同，这是由其架构决定的。

| 信息类型 | Hosted NIM | Local NIM | 说明 |
|---------|:----------:|:---------:|------|
| Function ID | ✅ | ❌ | 只有 Hosted NIM 运行在 NVCF 上 |
| Status (ACTIVE/INACTIVE) | ✅ | ❌ | Hosted NIM 是托管服务 |
| Container Image | ✅ | ❌ | Hosted NIM 的底层镜像 |
| Latest Tag → 实际版本 | ❌ | ✅ | Local NIM 是 Docker 镜像 |
| 镜像描述 | ❌ | ✅ | 来自 NGC 镜像仓库 |
| Inference URL | ✅ | ❌ | Hosted NIM 的 API 端点 |

### 10.5 环境变量

| 环境变量 | 说明 |
|---------|------|
| `NVIDIA_API_KEY` | NGC API Key（可代替 `--ngc-api-key` 参数） |
| `GITHUB_TOKEN` | GitHub Token（可代替 `--github-token` 参数，仅 scan 需要） |
| `RUST_LOG` | 日志级别（debug/info/warn/error） |

### 10.6 使用示例

```bash
# === scan 子命令 ===

# 基本扫描（通过环境变量提供参数）
export NVIDIA_API_KEY="nvapi-xxx"
export GITHUB_TOKEN="ghp_xxx"
nim-usage-scanner scan -c repos.yaml

# 指定输出目录
nim-usage-scanner scan -c repos.yaml -o ./reports

# 详细日志模式
nim-usage-scanner scan -c repos.yaml -vv

# === query 子命令 ===

# 查询 Hosted NIM 信息
nim-usage-scanner query hosted-nim \
  --model "nvidia/llama-3.1-nemotron-70b-instruct" \
  --ngc-api-key "nvapi-xxx"

# 查询 Local NIM 信息
nim-usage-scanner query local-nim \
  --image "nvidia/llama-3.2-nv-embedqa-1b-v2" \
  --ngc-api-key "nvapi-xxx"

# 查询时带完整镜像路径也可以
nim-usage-scanner query local-nim \
  --image "nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2" \
  --ngc-api-key "nvapi-xxx"
```
