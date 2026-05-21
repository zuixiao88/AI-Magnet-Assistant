// src-tauri/src/llm_service.rs

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

/// 智能处理API Base URL，为不同的API服务添加正确的路径
fn normalize_api_base(api_base: &str) -> String {
    // 移除末尾的斜杠，避免双斜杠问题
    let trimmed_base = api_base.trim_end_matches('/');

    // 如果未包含 /v1beta，针对官方域名与通用 http(s) 代理统一追加 /v1beta
    if !trimmed_base.contains("/v1beta")
        && (
            trimmed_base == "https://generativelanguage.googleapis.com"
                || trimmed_base.starts_with("https://generativelanguage.googleapis.com")
                || ((trimmed_base.starts_with("http://") || trimmed_base.starts_with("https://"))
                    && !trimmed_base.contains("/api/")
                    && !trimmed_base.contains("/v1/"))
        )
    {
        format!("{trimmed_base}/v1beta")
    } else {
        // 对于其他URL（包括已经包含路径的自定义代理），保持原样但移除末尾斜杠
        trimmed_base.to_string()
    }
}

fn safe_api_url(normalized_base: &str, model: &str) -> String {
    format!("{normalized_base}/models/{model}:generateContent")
}

fn compact_error_body(error_body: &str) -> String {
    const MAX_ERROR_BODY_CHARS: usize = 500;

    let compact = error_body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if compact.chars().count() > MAX_ERROR_BODY_CHARS {
        format!(
            "{}...",
            compact.chars().take(MAX_ERROR_BODY_CHARS).collect::<String>()
        )
    } else {
        compact
    }
}

fn api_error_message(status: StatusCode, error_body: &str) -> String {
    let body = compact_error_body(error_body);
    if body.is_empty() {
        format!("API请求失败 (状态码: {status})")
    } else {
        format!("API请求失败 (状态码: {status}): {body}")
    }
}

// --- 0. 公共配置 ---

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LlmConfig {
    pub provider: String,
    pub api_key: String,
    pub api_base: String,
    pub model: String,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
}

fn default_batch_size() -> u32 {
    5
}

// --- 1. 第一阶段：从HTML中提取基础信息 ---

/// 第一阶段：从HTML中提取的单个原始、未经处理的磁力链接信息
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExtractedBasicInfo {
    pub title: String,
    pub magnet_link: String,
    pub file_size: Option<String>,
    pub source_url: Option<String>,
}

/// 第一阶段：批量提取结果
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BatchExtractBasicInfoResult {
    pub results: Vec<ExtractedBasicInfo>,
}

// --- 2. 第二阶段：分析分数和标签 ---

/// 第二阶段：对单个磁力链接的文件列表进行详细分析后的最终结果
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DetailedAnalysisResult {
    pub title: String,           // 精简后的标题
    pub purity_score: u8,        // 纯净度分数 (由LLM计算)
    pub tags: Vec<String>,       // 智能标签
    pub magnet_link: String,     // 原始磁力链接 (从第一阶段透传)
    pub file_size: Option<String>, // 原始文件大小 (从第一阶段透传)
    pub file_list: Vec<String>, // 文件列表
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,   // 错误信息 (如果分析失败)
}

// （已移除未使用的 LlmFileAnalysis 结构体）

/// 批量分析的输入项
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BatchAnalysisItem {
    pub title: String,
    pub file_list: Vec<String>,
}

/// 批量分析的结果项
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BatchAnalysisResult {
    pub cleaned_title: String,
    pub purity_score: u8,
    pub tags: Vec<String>,
}

// --- 3. LLM客户端定义 ---

#[async_trait]
pub trait LlmClient: Send + Sync {
    /// 第一阶段：从HTML页面批量提取基础、原始的磁力链接信息
    async fn batch_extract_basic_info_from_html(
        &self,
        html_content: &str,
        extraction_config: &LlmConfig,
    ) -> Result<BatchExtractBasicInfoResult>;

    /// 第二阶段：根据文件列表批量分析分数和标签（单个项目）
    async fn batch_analyze_scores_and_tags(
        &self,
        original_title: &str,
        file_list: &[String],
        analysis_config: &LlmConfig,
    ) -> Result<(String, u8, Vec<String>)>;

    /// 第二阶段：真正的批量分析多个项目
    async fn batch_analyze_multiple_items(
        &self,
        items: &[BatchAnalysisItem],
        analysis_config: &LlmConfig,
    ) -> Result<Vec<BatchAnalysisResult>>;
}

pub struct GeminiClient {
    client: Client,
}

impl GeminiClient {
    pub fn new() -> Self {
        let client = Client::new();
        Self { client }
    }
}

impl Default for GeminiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmClient for GeminiClient {
    async fn batch_extract_basic_info_from_html(
        &self,
        html_content: &str,
        extraction_config: &LlmConfig,
    ) -> Result<BatchExtractBasicInfoResult> {
        self.batch_extract_basic_info_impl(html_content, extraction_config).await
    }

    async fn batch_analyze_scores_and_tags(
        &self,
        original_title: &str,
        file_list: &[String],
        analysis_config: &LlmConfig,
    ) -> Result<(String, u8, Vec<String>)> {
        self.batch_analyze_scores_and_tags_impl(original_title, file_list, analysis_config)
            .await
    }

    async fn batch_analyze_multiple_items(
        &self,
        items: &[BatchAnalysisItem],
        analysis_config: &LlmConfig,
    ) -> Result<Vec<BatchAnalysisResult>> {
        self.batch_analyze_multiple_items_impl(items, analysis_config).await
    }
}

// --- 4. Gemini API请求和响应结构 ---

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<Content>,
}

#[derive(Serialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Serialize)]
struct Part {
    text: String,
}

#[derive(Deserialize, Debug)]
struct GeminiResponse {
    candidates: Vec<Candidate>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    content: ContentResponse,
}

#[derive(Deserialize, Debug)]
struct ContentResponse {
    parts: Vec<PartResponse>,
}

#[derive(Deserialize, Debug)]
struct PartResponse {
    text: String,
}

// --- 5. 核心实现 ---

impl GeminiClient {
    /// **第一阶段实现**: 仅从HTML提取原始数据，不做任何修改。
    async fn batch_extract_basic_info_impl(
        &self,
        html_content: &str,
        config: &LlmConfig,
    ) -> Result<BatchExtractBasicInfoResult> {
        let normalized_base = normalize_api_base(&config.api_base);
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            normalized_base, config.model, config.api_key
        );

        let prompt = format!(
            r#"
作为数据提取引擎，你的唯一任务是从以下HTML内容中识别出所有磁力链接条目，并返回一个包含 "results" 数组的JSON对象。

**重要提示**: 如果HTML内容包含乱码字符(�)或看起来不是正常的HTML，请仍然尝试提取任何可识别的磁力链接。

**提取规则:**
1.  **识别条目**: 找到包含磁力链接 (`magnet:?xt=`) 的HTML片段。磁力链接通常在以下位置：
    - `<a href="magnet:?xt=urn:btih:...">` 标签中
    - 可能在各种HTML结构中，如表格、列表、div等
2.  **提取字段**:
    *   `title`: 提取与磁力链接相关的最直接的标题文本。**重要：移除所有HTML标签（如<b>、<em>、<strong>等），只返回纯文本内容**。
    *   `magnet_link`: 提取完整的磁力链接字符串，必须以 `magnet:?xt=` 开头。
    *   `file_size`: 提取与该条目相关的文件大小文本（例如 "1.5GB", "899MB", "78.78G"）。如果找不到，则返回 `null`。
    *   `source_url`: 提取与该条目相关的详情页面链接或源页面URL。通常是标题链接的href属性。如果找不到，则返回 `null`。
3.  **严格JSON输出**: 返回的JSON对象必须只包含一个 `results` 键，其值为一个对象数组。每个对象都包含 `title`, `magnet_link`, `file_size`, `source_url` 字段。

**如果找不到任何磁力链接，请返回空数组但仍要说明原因**。

**重要指令:**
*   **绝对禁止修改数据**: 你的任务是提取，不是处理。返回你找到的原始信息。
*   **无需理解内容**: 不要尝试理解标题的含义或美化它。
*   **保持顺序**: 尽可能按照在HTML中出现的顺序列出结果。
*   **不要包含任何解释**: 你的输出必须是纯粹的JSON。

**HTML内容:**
```html
{}
```

**示例输出:**
```json
{{
  "results": [
    {{
      "title": "Some.Movie.Title.2023.1080p.BluRay.x264-GROUP[rartv]",
      "magnet_link": "magnet:?xt=urn:btih:abcdef123456...",
      "file_size": "2.3GB",
      "source_url": "/details/12345"
    }},
    {{
      "title": "[AD] www.example.com [AD] Another.Show.S01E01.720p.WEB-DL",
      "magnet_link": "magnet:?xt=urn:btih:fedcba654321...",
      "file_size": "500MB",
      "source_url": "https://example.com/torrent/67890"
    }}
  ]
}}
```
"#,
            html_content
        );

        let request_body = GeminiRequest {
            contents: vec![Content {
                parts: vec![Part { text: prompt }],
            }],
        };

        let response = self.client.post(&url).json(&request_body).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            eprintln!(
                "API请求失败: {} - {}",
                status,
                compact_error_body(&error_body)
            );
            return Err(anyhow::anyhow!(api_error_message(status, &error_body)));
        }

        let gemini_response = response.json::<GeminiResponse>().await?;
        if let Some(candidate) = gemini_response.candidates.first() {
            if let Some(part) = candidate.content.parts.first() {
                let cleaned_text = part.text.trim().replace("```json", "").replace("```", "");
                let result: BatchExtractBasicInfoResult = serde_json::from_str(&cleaned_text)
                    .map_err(|e| {
                        eprintln!("第一阶段JSON解析失败: {e}");
                        anyhow::anyhow!(
                            "解析第一阶段JSON失败: {}. Raw text: {}",
                            e,
                            compact_error_body(&cleaned_text)
                        )
                    })?;
                return Ok(result);
            }
        }
        Err(anyhow::anyhow!("Gemini响应中未找到有效内容"))
    }

    /// **重构后的第二阶段实现**: 根据新的、更简单的逻辑分析标题、文件列表和标签（支持重试）。
    async fn batch_analyze_scores_and_tags_impl(
        &self,
        original_title: &str,
        file_list: &[String],
        config: &LlmConfig,
    ) -> Result<(String, u8, Vec<String>)> {
        println!("🔧 [DEBUG] Starting single analysis for '{}' using batch method, batch_size={}",
                 original_title, config.batch_size);

        // 转换为批量格式（单个项目）
        let items = vec![BatchAnalysisItem {
            title: original_title.to_string(),
            file_list: file_list.to_vec(),
        }];

        // 调用批量分析（包含重试机制）
        let results = self.batch_analyze_multiple_items_impl(&items, config).await?;

        // 提取第一个结果
        if let Some(result) = results.first() {
            println!("✅ [DEBUG] Single analysis via batch method succeeded");
            Ok((result.cleaned_title.clone(), result.purity_score, result.tags.clone()))
        } else {
            Err(anyhow::anyhow!("批量分析未返回结果"))
        }
    }

    // try_single_analyze_scores_and_tags 方法已被移除
    // 现在统一使用 try_batch_analyze_multiple_items 处理单个和批量分析

    /// 真正的批量分析实现，支持重试机制
    async fn batch_analyze_multiple_items_impl(
        &self,
        items: &[BatchAnalysisItem],
        config: &LlmConfig,
    ) -> Result<Vec<BatchAnalysisResult>> {
        let mut retry_count = 0;
        const MAX_RETRIES: u32 = 3;
        const RETRY_DELAY_SECONDS: u64 = 3;

        println!("🔧 [DEBUG] Starting batch analysis with {} items, batch_size={}",
                 items.len(), config.batch_size);

        loop {
            println!("🔧 [DEBUG] Attempt {} of {}", retry_count + 1, MAX_RETRIES + 1);
            match self.try_batch_analyze_multiple_items(items, config).await {
                Ok(results) => {
                    println!("✅ [DEBUG] Batch analysis succeeded on attempt {}", retry_count + 1);
                    return Ok(results);
                }
                Err(e) => {
                    retry_count += 1;
                    println!("❌ [DEBUG] Batch analysis failed on attempt {retry_count}: {e}");

                    if retry_count >= MAX_RETRIES {
                        println!("💥 [DEBUG] Max retries reached, giving up");
                        return Err(anyhow::anyhow!("批量分析失败，已重试{}次: {}", MAX_RETRIES, e));
                    }

                    println!("⚠️ 批量分析失败，{RETRY_DELAY_SECONDS}秒后重试 ({retry_count}/{MAX_RETRIES}): {e}");

                    tokio::time::sleep(tokio::time::Duration::from_secs(RETRY_DELAY_SECONDS)).await;
                    println!("🔄 [DEBUG] Retrying now...");
                }
            }
        }
    }

    /// 尝试批量分析（不包含重试逻辑）
    async fn try_batch_analyze_multiple_items(
        &self,
        items: &[BatchAnalysisItem],
        config: &LlmConfig,
    ) -> Result<Vec<BatchAnalysisResult>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }

        let normalized_base = normalize_api_base(&config.api_base);
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            normalized_base, config.model, config.api_key
        );

        // 构建批量分析的 prompt
        let items_json = serde_json::to_string_pretty(items)?;

        let prompt = format!(
            r#"
作为媒体资源批量分析引擎，请对以下多个项目进行分析。对每个项目，你需要根据以下三项独立任务进行分析，并严格按照JSON格式返回结果。

**任务1：精简标题**
- **输入**: 原始标题字符串。
- **规则**:
  1. 仅输出作品名称和剧集信息，移除所有其他内容（广告、网址、推广信息、画质、格式等）。
  2. 作品名称：如有多个作品名称或多个语言版本，按英语 → 汉语 → 其他语言的顺序全部输出，用空格分隔。
  3. 剧集信息：如有多个季数或集数，全部输出（如同时有第二季和第三季输出S02 S03，同时有第二季第三集和第一季第二集输出S01E02 S02E03），如原始标题中没有显示则不输出。
  4. 格式：作品名称（多个名称用空格分隔）+ 剧集信息（多个季集用空格分隔），中间用空格分隔。
- **输出**: 返回精简后的标题字符串。

**任务2：计算纯净度分数**
- **输入**: 文件名列表 (JSON Array)。
- **规则**:
  1. 遍历列表中的每个文件名。
  2. 根据以下标准为每个文件打分：
     - **0分**: 纯广告文件（如 `.txt`, `.url`, 或包含明确广告词语的文件）。
     - **80分**: 文件名包含广告信息（如网址）的媒体资源文件。
     - **100分**: 文件名干净、不含任何广告信息的媒体资源文件。
  3. 计算所有文件分数的**平均值**，并四舍五入为整数。
- **输出**: 返回一个0-100之间的整数作为最终纯净度分数。

**任务3：提取标签**
- **输入**: 原始标题字符串。
- **规则**:
  1. **严格按顺序**提取以下4类标签，每类最多1个，总共最多4个标签：
     - **画质**: 使用标准格式（如720p、1080p、4K、8K等）
     - **语言**: 使用英语输出（如Chinese、Korean、Japanese、English等）
     - **字幕**: 按字幕语言输出（如Chinese Sub、English Sub、Korean Sub等）
     - **特殊格式**: 使用英语输出（如BluRay、Dolby、HDR、DV等）
  2. 如果某类信息无法从原始标题中获取，该位置留空，不要编造。
  3. 严格按照上述顺序排列，最多输出4个标签。
- **输出**: 返回包含标签的字符串数组，最多4个元素。

**输入数据**:
```json
{}
```

**输出要求**:
- 严格按照以下JSON格式返回，不要包含任何额外的解释或Markdown标记。
- results数组中的每个对象对应输入中的一个项目（按相同顺序）。
- `cleaned_title` 对应任务1的输出。
- `purity_score` 对应任务2的输出。
- `tags` 对应任务3的输出。

**示例输出:**
```json
{{
  "results": [
    {{
      "cleaned_title": "Transformers Batman 变形金刚 蝙蝠侠 S01E02 S02E03",
      "purity_score": 95,
      "tags": ["4K", "Chinese", "Chinese Sub", "BluRay"]
    }}
  ]
}}
```
"#,
            items_json
        );

        // 移除详细的Prompt日志以简化输出
        // println!("[BATCH AI PROMPT] 批量分析prompt:\n---\n{}\n---", prompt);

        let request_body = GeminiRequest {
            contents: vec![Content {
                parts: vec![Part { text: prompt }],
            }],
        };

        let response = self.client.post(&url).json(&request_body).send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(api_error_message(status, &error_body)));
        }

        let gemini_response = response.json::<GeminiResponse>().await?;
        if let Some(candidate) = gemini_response.candidates.first() {
            if let Some(part) = candidate.content.parts.first() {
                let cleaned_text = part.text.trim().replace("```json", "").replace("```", "");

                // 移除详细的响应日志以简化输出
                // println!("[BATCH AI RESPONSE] 批量分析响应:\n---\n{}\n---", cleaned_text);

                #[derive(Deserialize)]
                struct BatchAnalysisResponse {
                    results: Vec<BatchAnalysisResult>,
                }

                let batch_response: BatchAnalysisResponse = serde_json::from_str(&cleaned_text)
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "解析批量分析响应JSON失败: {}. Raw text: {}",
                            e,
                            compact_error_body(&cleaned_text)
                        )
                    })?;

                // 验证结果数量是否匹配
                if batch_response.results.len() != items.len() {
                    return Err(anyhow::anyhow!(
                        "批量分析结果数量不匹配: 期望{}, 实际{}",
                        items.len(),
                        batch_response.results.len()
                    ));
                }

                return Ok(batch_response.results);
            }
        }
        Err(anyhow::anyhow!("Gemini响应中未找到有效内容"))
    }
}

// --- 6. 公共API函数 ---
// 注意：原有的公共API函数已被删除，因为它们未被使用
// 所有AI调用现在都通过LlmClient trait进行

/// 测试与LLM提供商的连接。
pub async fn test_connection(config: &LlmConfig) -> Result<String> {
    let normalized_base = normalize_api_base(&config.api_base);
    let url = format!(
        "{}/models/{}:generateContent?key={}",
        normalized_base, config.model, config.api_key
    );

    // 简化调试信息
    println!("Testing connection to: {}", safe_api_url(&normalized_base, &config.model));
    let request_body = GeminiRequest {
        contents: vec![Content {
            parts: vec![Part {
                text: "你好".to_string(),
            }],
        }],
    };
    let client = Client::new();
    let response = client.post(&url).json(&request_body).send().await?;

    let status = response.status();
    if status.is_success() {
        println!("✅ Connection successful (Status: {status}).");
        Ok("连接成功".to_string())
    } else {
        let error_body = response.text().await.unwrap_or_default();
        println!(
            "Connection failed (Status: {status}): {}",
            compact_error_body(&error_body)
        );

        // 为常见错误提供更友好的提示
        let error_message = match status.as_u16() {
            401 => "认证失败：请检查API Key是否正确".to_string(),
            403 => "访问被拒绝：请检查API Key权限".to_string(),
            404 => "API路径不存在：请检查API Base URL是否正确".to_string(),
            405 => "请求方法不允许：API路径可能不正确".to_string(),
            500 => "服务器内部错误：可能是API Key无效或模型名称错误".to_string(),
            _ => format!("API连接失败 (状态码: {status})"),
        };

        let body = compact_error_body(&error_body);
        if body.is_empty() {
            Err(anyhow::anyhow!(error_message))
        } else {
            Err(anyhow::anyhow!("{}: {}", error_message, body))
        }
    }
}
