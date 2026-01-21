# NIM Usage Scanner 实施指南

## 1. 项目初始化

### 1.1 创建 Rust 项目

在 `nim-usage-scanner` 目录下执行 `cargo init`，创建一个名为 `nim-usage-scanner` 的二进制项目。

### 1.2 配置 Cargo.toml

添加以下依赖：

| 依赖 | 版本 | 用途 |
|------|------|------|
| `clap` | 4.x | 命令行参数解析，启用 `derive` feature |
| `serde` | 1.x | 序列化框架，启用 `derive` feature |
| `serde_json` | 1.x | JSON 输出 |
| `serde_yaml` | 0.9.x | YAML 配置解析 |
| `regex` | 1.x | 正则表达式匹配 |
| `ignore` | 0.4.x | 智能文件遍历（ripgrep 核心库） |
| `rayon` | 1.x | 并行处理 |
| `reqwest` | 0.12.x | HTTP 客户端，启用 `blocking` 和 `json` feature |
| `csv` | 1.x | CSV 输出 |
| `chrono` | 0.4.x | 时间处理，启用 `serde` feature |
| `tempfile` | 3.x | 临时目录管理 |
| `anyhow` | 1.x | 错误处理 |
| `log` | 0.4.x | 日志接口 |
| `env_logger` | 0.11.x | 日志实现 |

### 1.3 创建目录结构

```
nim-usage-scanner/
├── Cargo.toml
├── src/
│   ├── main.rs           # CLI 入口
│   ├── config.rs         # 配置加载
│   ├── git_ops.rs        # Git 操作
│   ├── scanner.rs        # 扫描逻辑
│   ├── ngc_api.rs        # NGC API 客户端
│   ├── report.rs         # 报告生成
│   └── models.rs         # 数据模型
├── config/
│   └── repos.yaml        # 配置文件示例
└── docs/
    └── zh/
        ├── architecture-design.md
        └── implementation-guide.md
```

---

## 2. 模块实现顺序

按照依赖关系，建议按以下顺序实现：

```
┌─────────────────────────────────────────────────────────────────┐
│  Phase 1: 基础模块                                               │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐         │
│  │  models.rs  │ ─▶ │  config.rs  │ ─▶ │  git_ops.rs │         │
│  │  (数据结构)  │    │  (配置解析)  │    │  (仓库克隆)  │         │
│  └─────────────┘    └─────────────┘    └─────────────┘         │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 2: 核心扫描                                               │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                      scanner.rs                          │    │
│  │  • 文件遍历逻辑                                           │    │
│  │  • Local NIM 正则匹配                                     │    │
│  │  • Hosted NIM 正则匹配                                    │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 3: API 集成                                               │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                      ngc_api.rs                          │    │
│  │  • NGC API 认证                                          │    │
│  │  • Function 查询                                         │    │
│  │  • 结果缓存                                              │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Phase 4: 输出与集成                                             │
│  ┌─────────────┐    ┌─────────────┐                            │
│  │  report.rs  │ ─▶ │  main.rs    │                            │
│  │  (报告生成)  │    │  (CLI集成)   │                            │
│  └─────────────┘    └─────────────┘                            │
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. Phase 1: 基础模块实现

### 3.1 models.rs - 数据模型

**任务 3.1.1：定义配置数据结构**

定义以下结构体用于解析 `repos.yaml`：

| 结构体 | 字段 | 说明 |
|--------|------|------|
| `Config` | `version`, `defaults`, `repos` | 顶层配置 |
| `Defaults` | `branch`, `depth` | 默认值配置 |
| `RepoConfig` | `name`, `url`, `branch`, `depth`, `enabled` | 单个仓库配置 |

所有结构体需实现 `Deserialize` trait。

**任务 3.1.2：定义来源类型枚举**

定义 `SourceType` 枚举用于内部分类：

| 枚举值 | 说明 | 判断条件 |
|--------|------|---------|
| `SourceCode` | 普通源代码 | 文件路径不匹配 `.github/workflows/*.yml` |
| `ActionsWorkflow` | Actions Workflow | 文件路径匹配 `.github/workflows/*.yml` 或 `.github/workflows/*.yaml` |

**任务 3.1.3：定义扫描结果数据结构**

定义以下结构体用于存储扫描结果：

| 结构体 | 字段 | 说明 |
|--------|------|------|
| `LocalNimMatch` | `repository`, `image_url`, `tag`, `resolved_tag`, `file_path`, `line_number`, `match_context` | Local NIM 匹配结果（`resolved_tag` 为 NGC API 解析的实际版本） |
| `HostedNimMatch` | `repository`, `endpoint_url`, `model_name`, `file_path`, `line_number`, `match_context`, `function_id`, `status`, `container_image` | Hosted NIM 匹配结果 |
| `NimFindings` | `local_nim`, `hosted_nim` | 某一来源类型下的 NIM 结果集 |
| `ScanReport` | `scan_time`, `total_repos`, `source_code`, `actions_workflow`, `aggregated`, `summary` | 完整报告（顶层按来源分类，含聚合视图） |
| `Summary` | `total_local_nim`, `total_hosted_nim`, `repos_with_nim`, `source_code`, `actions_workflow` | 统计摘要（含分类统计） |
| `CategorySummary` | `local_nim`, `hosted_nim` | 单个来源类型的统计 |

**数据结构层级**：

```
ScanReport
├── source_code: NimFindings
│   ├── local_nim: Vec<LocalNimMatch>
│   └── hosted_nim: Vec<HostedNimMatch>
├── actions_workflow: NimFindings
│   ├── local_nim: Vec<LocalNimMatch>
│   └── hosted_nim: Vec<HostedNimMatch>
├── aggregated: AggregatedFindings
│   ├── local_nim: Vec<AggregatedLocalNim>  // 按 image_url+tag 聚合
│   │   └── locations: Vec<NimLocation>     // 所有出现位置
│   └── hosted_nim: Vec<AggregatedHostedNim>  // 按 model_name 聚合
│       └── locations: Vec<NimLocation>
└── summary: Summary
    ├── source_code: CategorySummary
    └── actions_workflow: CategorySummary
```

所有结构体需实现 `Serialize` trait。

**任务 3.1.4：定义 NGC API 响应数据结构**

定义以下结构体用于解析 NGC API 响应：

| 结构体 | 用途 |
|--------|------|
| `NgcRepoResponse` | Container Registry API 响应（用于 latest tag 解析） |
| `NgcFunctionListResponse` | 函数列表 API 响应 |
| `NgcFunctionDetails` | 函数详情 |

### 3.2 config.rs - 配置加载

**任务 3.2.1：实现配置文件读取**

实现 `load_config` 函数：
- 输入：配置文件路径
- 输出：`Result<Config>`
- 功能：读取 YAML 文件，解析为 `Config` 结构体

**任务 3.2.2：实现配置验证**

实现 `validate_config` 函数：
- 检查所有仓库 URL 格式是否正确（以 `https://` 或 `git@` 开头）
- 检查仓库名称是否唯一
- 返回验证错误列表

**任务 3.2.3：实现默认值合并**

实现 `apply_defaults` 函数：
- 对每个仓库配置，如果 `branch` 或 `depth` 为空，使用 `defaults` 中的值
- 如果 `defaults` 也为空，使用硬编码默认值（branch: "main", depth: 1）

### 3.3 git_ops.rs - Git 操作

**任务 3.3.1：实现仓库克隆**

实现 `clone_repo` 函数：
- 输入：`RepoConfig`, 目标目录路径
- 输出：`Result<PathBuf>`（克隆后的目录路径）
- 功能：
  1. 构建 git clone 命令
  2. 添加 `--depth` 参数
  3. 添加 `--branch` 参数（如果指定）
  4. 执行命令并检查返回码
  5. 返回克隆目录路径

**任务 3.3.2：实现批量克隆**

实现 `clone_all_repos` 函数：
- 输入：仓库配置列表，工作目录
- 输出：`Vec<(RepoConfig, Result<PathBuf>)>`
- 功能：
  1. 创建工作目录（如果不存在）
  2. 使用 rayon 并行克隆所有仓库
  3. 收集每个仓库的克隆结果（成功或失败）

**任务 3.3.3：实现目录清理**

实现 `cleanup_repos` 函数：
- 输入：工作目录路径
- 输出：`Result<()>`
- 功能：递归删除工作目录

---

## 4. Phase 2: 核心扫描实现

### 4.1 scanner.rs - 扫描逻辑

**任务 4.1.1：定义扫描正则表达式**

创建两组预编译正则表达式：

**Local NIM 正则**：

| 模式 ID | 匹配目标 | 捕获组 |
|---------|---------|--------|
| `LOCAL_FULL` | `nvcr.io/nim/namespace/name:tag` | 1: namespace/name, 2: tag |
| `LOCAL_NO_TAG` | `nvcr.io/nim/namespace/name` | 1: namespace/name |

**Hosted NIM 正则**：

| 模式 ID | 匹配目标 | 捕获组 |
|---------|---------|--------|
| `ENDPOINT_URL` | `https://*.api.nvidia.com/*` | 1: 完整 URL |
| `MODEL_ASSIGN` | `model = "xxx"` 或 `model: "xxx"` | 1: model name |
| `CHATNVIDIA` | `ChatNVIDIA(model="xxx")` | 1: model name |

使用 `lazy_static` 或 `once_cell` 在编译时创建正则对象。

**任务 4.1.2：实现文件遍历**

实现 `scan_directory` 函数：
- 输入：目录路径，仓库名称
- 输出：`(Vec<LocalNimMatch>, Vec<HostedNimMatch>)`
- 功能：
  1. 使用 `ignore` crate 的 `WalkBuilder` 遍历目录
  2. 自动尊重 `.gitignore` 规则
  3. 添加自定义忽略规则（node_modules, vendor 等）
  4. 过滤文件扩展名（仅处理 .py, .yaml, .yml, .sh, Dockerfile 等）
  5. 对每个文件调用扫描函数

**任务 4.1.3：实现单文件扫描**

实现 `scan_file` 函数：
- 输入：文件路径，仓库名称
- 输出：`(Vec<LocalNimMatch>, Vec<HostedNimMatch>)`
- 功能：
  1. 读取文件内容
  2. 逐行遍历，记录行号
  3. 对每行应用所有正则表达式
  4. 提取匹配结果，构建 Match 对象
  5. 返回所有匹配

**任务 4.1.4：实现来源类型判断**

实现 `determine_source_type` 函数：
- 输入：文件相对路径
- 输出：`SourceType`
- 功能：
  1. 判断路径是否匹配 `.github/workflows/*.yml` 或 `.github/workflows/*.yaml`
  2. 如果匹配，返回 `SourceType::Workflow`
  3. 否则返回 `SourceType::Source`

**判断逻辑**：
```
路径以 ".github/workflows/" 开头
  且
  (路径以 ".yml" 结尾 或 路径以 ".yaml" 结尾)
```

**任务 4.1.5：实现 Local NIM 提取**

实现 `extract_local_nim` 函数：
- 输入：行内容，行号，文件路径，仓库名称
- 输出：`Option<LocalNimMatch>`
- 功能：
  1. 尝试匹配 `LOCAL_FULL` 正则
  2. 如果匹配，提取 image_url 和 tag
  3. 如果不匹配，尝试 `LOCAL_NO_TAG`，tag 设为 "latest"
  4. 构建并返回 `LocalNimMatch`

**任务 4.1.6：实现 Hosted NIM 提取**

实现 `extract_hosted_nim` 函数：
- 输入：行内容，行号，文件路径，仓库名称
- 输出：`Vec<HostedNimMatch>`（一行可能有多个匹配）
- 功能：
  1. 尝试匹配 `ENDPOINT_URL` 正则，提取 endpoint
  2. 尝试匹配 `MODEL_ASSIGN` 正则，提取 model
  3. 尝试匹配 `CHATNVIDIA` 正则，提取 model
  4. 合并同一行的 endpoint 和 model
  5. 构建并返回 `HostedNimMatch` 列表

**任务 4.1.7：实现扫描结果分类**

实现 `categorize_results` 函数：
- 输入：`Vec<LocalNimMatch>`, `Vec<HostedNimMatch>`
- 输出：`(NimFindings, NimFindings)`（source_code, actions_workflow）
- 功能：
  1. 遍历所有 LocalNimMatch，调用 `determine_source_type` 判断来源
  2. 遍历所有 HostedNimMatch，调用 `determine_source_type` 判断来源
  3. 分别放入 source_code 或 actions_workflow 的 NimFindings 中
  4. 返回两个分类后的结果集

**任务 4.1.8：实现扫描结果去重**

实现 `deduplicate_results` 函数：
- 输入：`Vec<LocalNimMatch>`, `Vec<HostedNimMatch>`
- 输出：去重后的列表
- 功能：
  1. 根据 (repository, file_path, line_number) 去重
  2. 保留第一次出现的匹配

---

## 5. Phase 3: NGC API 集成

### 5.1 ngc_api.rs - NGC API 客户端

**任务 5.1.1：实现 API 客户端结构**

创建 `NgcClient` 结构体：
- 字段：
  - `api_key`: String
  - `client`: reqwest blocking Client
  - `local_nim_cache`: HashMap<String, String>（缓存 latest tag 解析结果）
  - `hosted_nim_cache`: HashMap<String, NgcFunctionDetails>（缓存 Function 详情）
- 功能：管理 API 认证和请求缓存

**任务 5.1.2：实现 Local NIM latest tag 解析**

实现 `NgcClient::resolve_latest_tag` 方法：
- 输入：image_url（如 "nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2"）
- 输出：`Result<String>`（实际版本号）
- 功能：
  1. 检查缓存，如果命中直接返回
  2. 从 image_url 解析 team 和 model-name
  3. 调用 NGC Container Registry API
  4. 从响应中提取 `latestTag` 字段
  5. 写入缓存
  6. 返回实际版本号

**NGC Container Registry API 端点**：
```
GET https://api.ngc.nvidia.com/v2/org/nim/team/{team}/repos/{model-name}
Authorization: Bearer <api_key>
```

**URL 构造规则**：
- 输入：`nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2`
- team = `nvidia`
- model-name = `llama-3.2-nv-embedqa-1b-v2`
- API URL = `https://api.ngc.nvidia.com/v2/org/nim/team/nvidia/repos/llama-3.2-nv-embedqa-1b-v2`

**响应解析**：
- 提取 `latestTag` 字段作为实际版本号
- 如果字段不存在，返回错误

**任务 5.1.3：实现 Function 搜索**

实现 `NgcClient::find_function_by_model` 方法：
- 输入：model name（如 "nvidia/llama-3.1-nemotron"）
- 输出：`Option<String>`（Function ID）
- 功能：
  1. 调用 NGC Functions List API
  2. 搜索 name 或 description 包含 model name 的函数
  3. 返回匹配的 Function ID

**NGC Functions List API 端点**：
```
GET https://api.nvcf.nvidia.com/v2/nvcf/functions
Authorization: Bearer <api_key>
```

**任务 5.1.4：实现 Function 详情获取**

实现 `NgcClient::get_function_details` 方法：
- 输入：Function ID
- 输出：`Result<NgcFunctionDetails>`
- 功能：
  1. 检查缓存，如果命中直接返回
  2. 调用 NGC Function Details API
  3. 解析响应，提取 status、containerImage 等字段
  4. 写入缓存
  5. 返回结果

**NGC Function Versions API 端点**（⚠️ 必须使用 `/versions` 端点）：
```
GET https://api.nvcf.nvidia.com/v2/nvcf/functions/{function_id}/versions
Authorization: Bearer <api_key>
```

> **重要**：直接访问 `/v2/nvcf/functions/{id}` 会返回 404。必须使用 `/versions` 端点获取详情。

**响应结构**：
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

**响应字段提取**（取 `functions[0]` 即最新版本）：

| 响应字段路径 | 输出字段 |
|-------------|---------|
| `functions[0].id` | `function_id` |
| `functions[0].status` | `status` |
| `functions[0].name` 或 `functions[0].models[0].name` | `model_name` |
| `functions[0].containerImage` | `container_image` |

**任务 5.1.5：实现 Local NIM 批量补充**

实现 `enrich_local_nim_matches` 函数：
- 输入：`&mut Vec<LocalNimMatch>`, `&NgcClient`
- 输出：无（原地修改）
- 功能：
  1. 筛选 tag 为 "latest" 或空的记录
  2. 对每条记录调用 `resolve_latest_tag`
  3. **将 `resolved_tag` 字段设置为实际版本号**（保留原始 `tag` 不变）
  4. 如果 API 调用失败，`resolved_tag` 保持为 `None` 并记录警告

**任务 5.1.6：实现 Hosted NIM 批量补充**

实现 `enrich_hosted_nim_matches` 函数：
- 输入：`&mut Vec<HostedNimMatch>`, `&NgcClient`
- 输出：无（原地修改）
- 功能：
  1. 收集所有唯一的 model_name
  2. 对每个 model_name 查询 NGC API
  3. 将获取到的 function_id、status、container_image 填充到对应的 Match 中

---

## 6. Phase 4: 输出与集成

### 6.1 report.rs - 报告生成

**任务 6.1.1：实现 JSON 报告生成**

实现 `generate_json_report` 函数：
- 输入：`ScanReport`，输出文件路径
- 输出：`Result<()>`
- 功能：
  1. 使用 serde_json 序列化报告
  2. 启用 pretty print（缩进 2 空格）
  3. 写入文件

**任务 6.1.2：实现 CSV 报告生成**

实现 `generate_csv_reports` 函数：
- 输入：`ScanReport`，输出目录
- 输出：`Result<()>`
- 功能：
  1. 创建 **统一的** `report.csv` 文件
  2. 写入表头：`source_type,nim_type,repository,file_path,line_number,image_url,tag,resolved_tag,endpoint_url,model_name,function_id,status,container_image,match_context`
  3. 依次写入 source_code.local_nim、source_code.hosted_nim、actions_workflow.local_nim、actions_workflow.hosted_nim 的数据
  4. Local NIM 行的 Hosted NIM 字段（endpoint_url 等）留空
  5. Hosted NIM 行的 Local NIM 字段（image_url 等）留空
  6. 处理字段中的特殊字符（逗号、引号、换行）

**任务 6.1.3：实现报告摘要计算**

实现 `calculate_summary` 函数：
- 输入：`NimFindings`（source_code），`NimFindings`（actions_workflow）
- 输出：`Summary`
- 功能：
  1. 计算总 Local NIM 数量（source_code + actions_workflow）
  2. 计算总 Hosted NIM 数量（source_code + actions_workflow）
  3. 计算包含 NIM 的仓库数量（去重统计 repository 字段）
  4. 计算 source_code 的 CategorySummary（local_nim, hosted_nim 数量）
  5. 计算 actions_workflow 的 CategorySummary（local_nim, hosted_nim 数量）

### 6.2 main.rs - CLI 入口

**任务 6.2.1：定义命令行参数**

使用 clap derive 宏定义 `Args` 结构体：

| 参数 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `config` | PathBuf | 是 | - | 配置文件路径 |
| `output` | PathBuf | 否 | `./output` | 输出目录 |
| `ngc_api_key` | String | **是** | - | NGC API Key（必填，或使用 `NVIDIA_API_KEY` 环境变量） |
| `github_token` | String | **是** | - | GitHub Token（必填，或使用 `GITHUB_TOKEN` 环境变量，用于克隆私有仓库） |
| `workdir` | Option<PathBuf> | 否 | 系统临时目录 | 工作目录 |
| `keep_repos` | bool | 否 | false | 保留克隆的仓库 |
| `verbose` | u8 | 否 | 0 | 日志级别（-v, -vv） |
| `jobs` | Option<usize> | 否 | CPU 核心数 | 并发数 |

**任务 6.2.2：实现主函数流程**

实现 `main` 函数执行流程：

```
1. 初始化日志
   └── 根据 verbose 参数设置日志级别

2. 解析命令行参数
   └── 使用 clap 解析

3. 读取必需参数
   └── NGC API Key：优先使用 `--ngc-api-key`，其次 `NVIDIA_API_KEY` 环境变量（必填）
   └── GitHub Token：优先使用 `--github-token`，其次 `GITHUB_TOKEN` 环境变量（必填）

4. 加载配置
   └── 调用 config::load_config
   └── 调用 config::validate_config
   └── 调用 config::apply_defaults

5. 过滤启用的仓库
   └── 只处理 enabled = true 的仓库

6. 创建工作目录
   └── 如果指定了 workdir 使用指定目录
   └── 否则使用 tempfile 创建临时目录

7. 克隆仓库
   └── 调用 git_ops::clone_all_repos
   └── 记录失败的仓库

8. 扫描仓库
   └── 并行调用 scanner::scan_directory
   └── 收集所有结果

9. 调用 NGC API 补充信息
   └── 如果提供了 API Key
   └── 调用 ngc_api::enrich_local_nim_matches（解析 latest tag）
   └── 调用 ngc_api::enrich_hosted_nim_matches（获取 Function 详情）

10. 生成报告
    └── 调用 report::calculate_summary
    └── 构建 ScanReport
    └── 调用 report::generate_json_report
    └── 调用 report::generate_csv_reports

11. 清理
    └── 如果 keep_repos = false
    └── 调用 git_ops::cleanup_repos

12. 输出摘要
    └── 打印扫描结果统计
```

**任务 6.2.3：实现错误处理**

- 使用 `anyhow::Result` 统一错误类型
- 配置相关错误导致程序退出
- 单个仓库错误记录警告但继续执行
- 最终输出失败仓库列表

---

## 7. 配置文件示例

### 7.1 repos.yaml

在 `config/repos.yaml` 创建示例配置：

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

## 8. 测试方案

### 8.1 单元测试

**config.rs 测试**：
- 测试有效 YAML 配置解析
- 测试无效 YAML 格式错误处理
- 测试默认值合并逻辑

**scanner.rs 测试**：
- 测试 Local NIM 正则匹配各种格式
- 测试 Hosted NIM 正则匹配各种格式
- 测试边界情况（注释中的 URL、字符串中的 URL）
- 测试文件遍历是否正确忽略目录

**ngc_api.rs 测试**：
- 测试 API 响应解析
- 测试缓存功能
- 测试 API 错误处理

**report.rs 测试**：
- 测试 JSON 输出格式
- 测试 CSV 特殊字符转义
- 测试 Summary 计算

### 8.2 集成测试

创建 `tests/` 目录，包含：

1. **测试仓库准备**：创建包含已知 NIM 引用的测试文件
2. **端到端测试**：执行完整扫描流程，验证输出

### 8.3 手动测试

准备以下测试用例文件：

**test_dockerfile**：
```dockerfile
FROM nvcr.io/nim/nvidia/llama-3.2-nv-embedqa-1b-v2:1.10.0
```

**test_compose.yaml**：
```yaml
services:
  nim:
    image: nvcr.io/nim/nvidia/llama:latest
```

**test_python.py**：
```python
client = OpenAI(base_url="https://ai.api.nvidia.com/v1")
response = client.chat.completions.create(model="nvidia/llama-3.1-nemotron")
```

**test_workflow.yml**（放置在 `.github/workflows/` 目录下）：
```yaml
name: Deploy
jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - name: Pull NIM
        run: docker pull nvcr.io/nim/nvidia/nemo-retriever:24.08
```

**预期结果验证**：
- `test_dockerfile`, `test_compose.yaml`, `test_python.py` 的结果应出现在 `source_code` 分类下
- `test_workflow.yml` 的结果应出现在 `actions_workflow` 分类下
- CSV 文件：`source_code_local_nim.csv` 应包含 Dockerfile 和 compose 的结果
- CSV 文件：`actions_workflow_local_nim.csv` 应包含 workflow 的结果

---

## 9. 实施检查清单

### Phase 1 检查项

- [ ] Cargo.toml 配置完成，所有依赖添加
- [ ] models.rs 所有数据结构定义完成（包含 SourceType 枚举）
- [ ] config.rs 配置加载、验证、默认值合并功能完成
- [ ] git_ops.rs 克隆、批量克隆、清理功能完成
- [ ] 单元测试通过

### Phase 2 检查项

- [ ] 所有正则表达式定义并测试
- [ ] 文件遍历功能完成，正确忽略目录
- [ ] 单文件扫描功能完成
- [ ] **来源类型判断功能完成（determine_source_type）**
- [ ] Local NIM 提取功能完成
- [ ] Hosted NIM 提取功能完成
- [ ] **结果分类功能完成（categorize_results）**
- [ ] 去重功能完成
- [ ] 单元测试通过

### Phase 3 检查项

- [ ] NgcClient 结构体实现（双缓存：local_nim_cache + hosted_nim_cache）
- [ ] Local NIM latest tag 解析功能完成（调用 NGC Container Registry API）
- [ ] Function 搜索功能完成（调用 NVCF Functions List API）
- [ ] Function 详情获取功能完成（调用 NVCF Function Details API）
- [ ] Local NIM 批量补充功能完成（enrich_local_nim_matches）
- [ ] Hosted NIM 批量补充功能完成（enrich_hosted_nim_matches）
- [ ] API 错误处理完成（401/404/429/5xx）
- [ ] 单元测试通过

### Phase 4 检查项

- [ ] JSON 报告生成功能完成（顶层按来源分类）
- [ ] CSV 报告生成功能完成（4 个文件：source_code_*, actions_workflow_*）
- [ ] Summary 计算功能完成（包含分类统计）
- [ ] CLI 参数解析完成
- [ ] 主函数流程完成
- [ ] 错误处理完成
- [ ] 集成测试通过
- [ ] 手动测试通过

---

## 10. 注意事项

### 10.1 正则表达式编写

1. 注意转义特殊字符（`.` 需要写成 `\.`）
2. 使用非贪婪匹配避免过度匹配
3. 考虑大小写（Dockerfile vs dockerfile）
4. 处理引号变体（单引号、双引号）

### 10.2 文件编码

1. 假设所有文件为 UTF-8 编码
2. 遇到非法 UTF-8 字符时跳过该文件并记录警告

### 10.3 并发安全

1. 使用 Arc 包装共享数据
2. 使用 Mutex 保护可变状态
3. NGC API 客户端的缓存需要线程安全

### 10.4 性能考虑

1. 预编译正则表达式
2. 使用 BufReader 读取大文件
3. 避免不必要的字符串复制
4. 控制并发数避免 GitHub API 限流

### 10.5 安全考虑

1. 不在日志中输出 API Key
2. API Key 通过环境变量传递更安全
3. 验证克隆 URL 格式防止命令注入
