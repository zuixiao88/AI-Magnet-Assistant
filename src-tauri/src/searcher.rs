use anyhow::{Result, anyhow};
// 移除未使用的顶层导入（reqwest 已通过具体路径使用）
use scraper::{Html, Selector};
use futures::future::join_all;
use std::sync::Arc;
use std::collections::HashSet;
use crate::llm_service::{LlmClient, GeminiClient, LlmConfig};

// 统一的日志宏
macro_rules! search_log {
    (info, $($arg:tt)*) => {
        println!("🔍 {}", format!($($arg)*))
    };
    (success, $($arg:tt)*) => {
        println!("✅ {}", format!($($arg)*))
    };
    (warn, $($arg:tt)*) => {
        println!("⚠️ {}", format!($($arg)*))
    };
    (error, $($arg:tt)*) => {
        println!("❌ {}", format!($($arg)*))
    };
    (ai, $($arg:tt)*) => {
        println!("🤖 {}", format!($($arg)*))
    };
    (stats, $($arg:tt)*) => {
        println!("📊 {}", format!($($arg)*))
    };
}

// 统一的错误处理
fn handle_request_error(url: &str, error: reqwest::Error) -> anyhow::Error {
    search_log!(error, "Request failed for {}: {}", url, error);
    anyhow!("Request failed: {}", error)
}

fn is_cloudflare_challenge(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get("cf-mitigated")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("challenge"))
        .unwrap_or(false)
}

fn handle_http_status_error(url: &str, response: &reqwest::Response) -> anyhow::Error {
    let status = response.status();
    if status.as_u16() == 403 && is_cloudflare_challenge(response) {
        search_log!(error, "Cloudflare challenge blocked {}", url);
        anyhow!(
            "Search source blocked automated requests with Cloudflare challenge (HTTP 403): {}",
            url
        )
    } else {
        search_log!(error, "HTTP error {} for {}", status, url);
        anyhow!("HTTP error {}: {}", status, url)
    }
}

fn decode_html_entities(text: &str) -> String {
    text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&#61;", "=")
        .replace("&#x3D;", "=")
        .replace("&#38;", "&")
        .replace("&#x26;", "&")
        .replace("&nbsp;", " ")
}

/// 安全截断字符串，避免切到多字节字符中间
fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }

    // 找到不超过max_bytes的最大字符边界
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// 清理HTML标签和实体
fn clean_html_text(text: &str) -> String {
    // 移除HTML标签
    let re_tags = regex::Regex::new(r"<[^>]*>").unwrap();
    let text = re_tags.replace_all(text, "");

    // 解码常见的HTML实体
    let text = decode_html_entities(&text);

    // 清理多余的空格
    text.trim().replace("  ", " ")
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct SearchResult {
    pub title: String,
    pub magnet_link: String,
    pub file_size: Option<String>,
    pub upload_date: Option<String>,
    pub file_list: Vec<String>,
    pub source_url: Option<String>,
    pub preview_image_url: Option<String>,
    pub score: Option<u8>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
struct TorrentsCsvResponse {
    torrents: Vec<TorrentsCsvItem>,
}

#[derive(Debug, serde::Deserialize)]
struct TorrentsCsvItem {
    infohash: String,
    name: String,
    size_bytes: Option<u64>,
    created_unix: Option<i64>,
}

/// 搜索引擎提供商特性
#[async_trait::async_trait]
pub trait SearchProvider: Send + Sync {
    #[allow(dead_code)]
    fn name(&self) -> &str;
    async fn search(&self, query: &str, page: u32) -> Result<Vec<SearchResult>>;
}

/// clmclm.com 搜索引擎实现
pub struct ClmclmProvider {
    client: reqwest::Client,
    pub base_url: String,
}

impl ClmclmProvider {
    pub fn with_base_url(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub fn new() -> Self {
        Self::with_base_url("http://clmclm.com")
    }
}

#[async_trait::async_trait]
impl SearchProvider for ClmclmProvider {
    fn name(&self) -> &str {
        "clmclm.com"
    }

    async fn search(&self, query: &str, page: u32) -> Result<Vec<SearchResult>> {
        let encoded_query = urlencoding::encode(query);
        let url = format!("{}/search-{}-1-1-{}.html", self.base_url, encoded_query, page);
        search_log!(info, "Searching: {}", url);

        let response = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| handle_request_error(&url, e))?;

        if !response.status().is_success() {
            return Err(handle_http_status_error(&url, &response));
        }

        let html = response.text().await?;
        let results = self.parse_results(&html)?;
        search_log!(stats, "Found {} results on page {}", results.len(), page);
        Ok(results)
    }
}

impl ClmclmProvider {
    fn normalize_source_url(&self, href: &str) -> String {
        if href.starts_with("http://") || href.starts_with("https://") {
            href.to_string()
        } else if href.starts_with("//") {
            format!("https:{href}")
        } else if href.starts_with("/") {
            format!("{}{}", self.base_url, href)
        } else {
            href.to_string()
        }
    }

    fn extract_preview_image(&self, element: &scraper::ElementRef) -> Option<String> {
        let image_selector = Selector::parse("img").ok()?;

        element
            .select(&image_selector)
            .filter_map(extract_image_source)
            .find(|src| is_content_preview_image(src))
            .map(|src| self.normalize_source_url(src))
    }

    fn parse_results(&self, html: &str) -> Result<Vec<SearchResult>> {
        let document = Html::parse_document(html);

        let row_selector = Selector::parse("div.ssbox")
            .map_err(|e| anyhow!("Invalid CSS selector: {}", e))?;
        let title_selector = Selector::parse("div.title > h3 > a")
            .map_err(|e| anyhow!("Invalid CSS selector: {}", e))?;
        let magnet_selector = Selector::parse("div.sbar a[href^=\"magnet:\"]")
            .map_err(|e| anyhow!("Invalid CSS selector: {}", e))?;
        let file_list_selector = Selector::parse("ul > li")
            .map_err(|e| anyhow!("Invalid CSS selector: {}", e))?;

        let mut results = Vec::new();

        for element in document.select(&row_selector) {
            let title_element = element.select(&title_selector).next();
            let magnet_element = element.select(&magnet_selector).next();

            if let (Some(title_node), Some(magnet_node)) = (title_element, magnet_element) {
                let title = clean_html_text(&title_node.text().collect::<String>());
                let source_url = title_node.value().attr("href").map(|s| format!("{}{}", self.base_url, s));

                if let Some(magnet_link) = magnet_node.value().attr("href") {
                    // 尝试从所有span中找到文件大小
                    let mut file_size = None;
                    let span_selector = Selector::parse("div.sbar span").unwrap();
                    for span in element.select(&span_selector) {
                        let span_text = span.text().collect::<String>();
                        let span_text = span_text.trim();
                        if span_text.starts_with("大小:") {
                            file_size = Some(span_text.replace("大小:", "").trim().to_string());
                            break;
                        }
                    }

                    // 提取真实的文件列表
                    let mut file_list = Vec::new();
                    for li_element in element.select(&file_list_selector) {
                        let file_text = li_element.text().collect::<String>();
                        let file_text = file_text.trim();

                        // 解析文件名和大小，格式通常是 "文件名 大小"
                        if !file_text.is_empty() {
                            // 分割文件名和大小，大小通常在最后
                            let parts: Vec<&str> = file_text.split_whitespace().collect();
                            if parts.len() >= 2 {
                                // 检查最后一部分是否是文件大小（包含 GB, MB, KB 等）
                                let last_part = parts[parts.len() - 1];
                                if last_part.contains("GB") || last_part.contains("MB") || last_part.contains("KB") || last_part.contains("TB") {
                                    // 文件名是除了最后一部分的所有内容
                                    let filename = parts[..parts.len() - 1].join(" ");
                                    if !filename.is_empty() {
                                        file_list.push(filename);
                                    }
                                } else {
                                    // 如果没有识别到大小，就把整个文本作为文件名
                                    file_list.push(file_text.to_string());
                                }
                            } else {
                                // 如果只有一个部分，直接作为文件名
                                file_list.push(file_text.to_string());
                            }
                        }
                    }

                    // 如果没有解析到文件列表，使用基于标题的生成方法作为后备
                    if file_list.is_empty() {
                        file_list = self.extract_file_list_from_magnet(magnet_link, &title);
                    }

                    results.push(SearchResult {
                        title,
                        magnet_link: magnet_link.to_string(),
                        file_size,
                        upload_date: None, // clmclm.com doesn't provide upload date
                        file_list,
                        source_url,
                        preview_image_url: self.extract_preview_image(&element),
                        score: None,
                        tags: None,
                    });
                }
            }
        }

        Ok(results)
    }

    /// 从磁力链接和标题中提取文件列表（基于标题生成相关文件列表）
    fn extract_file_list_from_magnet(&self, magnet_link: &str, title: &str) -> Vec<String> {
        if !magnet_link.contains("btih:") {
            return vec![];
        }

        generate_file_list_from_title(title)
    }
}

/// 通用搜索引擎提供商，支持自定义URL模板和AI智能识别
pub struct GenericProvider {
    name: String,
    url_template: String,
    client: reqwest::Client,
    llm_client: Option<Arc<dyn LlmClient>>,
    extraction_config: Option<LlmConfig>,  // HTML提取配置（分析由前端处理）
    priority_keywords: Vec<String>,
}

impl GenericProvider {
    pub fn new(name: String, url_template: String) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .timeout(std::time::Duration::from_secs(30))
            // reqwest默认启用gzip/deflate解压，不需要显式设置
            .build()
            .expect("Failed to create HTTP client");

        Self {
            name,
            url_template,
            client,
            llm_client: None,
            extraction_config: None,
            priority_keywords: Vec::new(),
        }
    }

    /// 设置 LLM 客户端和（第一阶段 HTML 提取用的）配置
    pub fn with_llm_client_and_config(
        mut self,
        llm_client: Arc<dyn LlmClient>,
        extraction_config: LlmConfig,
    ) -> Self {
        self.llm_client = Some(llm_client);
        self.extraction_config = Some(extraction_config);
        self
    }

    /// 设置优先关键词用于匹配
    pub fn with_priority_keywords(mut self, keywords: Vec<String>) -> Self {
        self.priority_keywords = keywords;
        self
    }
}

#[async_trait::async_trait]
impl SearchProvider for GenericProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn search(&self, query: &str, page: u32) -> Result<Vec<SearchResult>> {
        // 替换URL模板中的占位符
        let encoded_query = urlencoding::encode(query);
        let mut url = self.url_template
            .replace("{keyword}", encoded_query.as_ref());

        // Handle different page numbering systems
        if url.contains("{page-1}") {
            // 0-based pagination: subtract 1 from page number
            let zero_based_page = if page > 0 { page - 1 } else { 0 };
            url = url.replace("{page-1}", &zero_based_page.to_string());
        } else {
            // 1-based pagination (default)
            url = url.replace("{page}", &page.to_string());
        }

        search_log!(info, "Searching: {}", url);

        let response = self.client
            .get(&url)
            .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Accept-Encoding", "gzip, deflate, br")
            .header("Cache-Control", "no-cache")
            .header("Pragma", "no-cache")
            .header("Sec-Ch-Ua", "\"Google Chrome\";v=\"131\", \"Chromium\";v=\"131\", \"Not_A Brand\";v=\"24\"")
            .header("Sec-Ch-Ua-Mobile", "?0")
            .header("Sec-Ch-Ua-Platform", "\"Windows\"")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Site", "cross-site")
            .header("Sec-Fetch-User", "?1")
            .header("Upgrade-Insecure-Requests", "1")
            .header("Referer", "https://www.google.com/")
            .send()
            .await
            .map_err(|e| handle_request_error(&url, e))?;

        if !response.status().is_success() {
            return Err(handle_http_status_error(&url, &response));
        }

        // 获取响应文本（reqwest自动处理压缩）
        let html = response.text().await
            .map_err(|e| anyhow!("Failed to read response: {}", e))?;

        // 检查响应内容类型
        let is_javascript = html.trim_start().starts_with("\"use strict\"") ||
                           html.contains("webpack") ||
                           html.contains("self.webpackChunk");

        if is_javascript {
            search_log!(warn, "网站返回JavaScript代码，可能是SPA或有反爬虫机制，跳过处理");
            return Ok(Vec::new());
        }

        if html.contains('�') {
            search_log!(warn, "HTML包含乱码字符，可能存在编码问题");
        }

        // 只在出现问题时显示HTML预览
        if html.contains('�') || is_javascript {
            let preview = safe_truncate(&html, 500);
            search_log!(info, "HTML preview (前500字符，用于诊断):");
            println!("---START---");
            println!("{preview}");
            println!("---END---");
        }

        // 简单检查内容
        let magnet_count = html.matches("magnet:").count();
        if magnet_count == 0 {
            let error_count = html.matches("404").count() + html.matches("Not Found").count();
            if error_count > 0 {
                search_log!(warn, "可能收到了错误页面，包含 {} 个错误指示符", error_count);
            }
        }

        let trimmed_html = html.trim_start();

        // JSON APIs are deterministic and do not need AI extraction.
        let results = if trimmed_html.starts_with('{') || trimmed_html.starts_with('[') {
            self.parse_json_results(&html)?
        } else if let Some(llm_client) = &self.llm_client {
            self.analyze_html_with_ai(&html, llm_client.clone()).await?
        } else {
            self.parse_generic_results(&html)?
        };

        search_log!(stats, "Found {} results on page {}", results.len(), page);
        Ok(results)
    }
}

impl GenericProvider {
    fn parse_json_results(&self, content: &str) -> Result<Vec<SearchResult>> {
        if let Ok(response) = serde_json::from_str::<TorrentsCsvResponse>(content) {
            let results = response
                .torrents
                .into_iter()
                .filter(|item| item.infohash.len() == 40)
                .map(|item| {
                    let magnet_link = format!(
                        "magnet:?xt=urn:btih:{}&dn={}",
                        item.infohash,
                        urlencoding::encode(&item.name)
                    );

                    SearchResult {
                        title: clean_html_text(&item.name),
                        magnet_link,
                        file_size: item.size_bytes.map(format_bytes),
                        upload_date: item.created_unix.and_then(format_unix_date),
                        file_list: generate_file_list_from_title(&item.name),
                        source_url: None,
                        preview_image_url: None,
                        score: None,
                        tags: None,
                    }
                })
                .collect();

            return Ok(results);
        }

        Err(anyhow!("Unsupported JSON search response format"))
    }

    /// 使用AI分析整个HTML内容
    async fn analyze_html_with_ai(&self, html: &str, llm_client: Arc<dyn LlmClient>) -> Result<Vec<SearchResult>> {
        search_log!(ai, "Phase 1: Extracting basic info from HTML...");

        let mut deterministic_results = self.parse_generic_results(html).unwrap_or_default();
        if deterministic_results.len() >= 5 {
            search_log!(
                success,
                "Basic parser extracted {} results. Skipping AI extraction to avoid result loss.",
                deterministic_results.len()
            );
            return Ok(deterministic_results);
        }

        // 第一阶段：让AI从HTML中提取所有磁力链接和基础信息
        match self.extract_torrents_from_html_with_ai(html, llm_client.clone()).await {
            Ok(mut results) => {
                if results.is_empty() {
                    search_log!(warn, "AI extraction found no results. Falling back to basic parsing");
                    return Ok(deterministic_results);
                }

                if !deterministic_results.is_empty() {
                    results.append(&mut deterministic_results);
                    let mut seen_magnets = HashSet::new();
                    results.retain(|result| seen_magnets.insert(result.magnet_link.clone()));
                }

                search_log!(ai, "Phase 2: Separating priority results...");
                let (priority_results, regular_results) = self.separate_priority_results(results);

                search_log!(success, "AI extraction completed: {} priority and {} regular results",
                         priority_results.len(), regular_results.len());

                // 合并结果：优先结果在前，普通结果在后
                let mut final_results = priority_results;
                final_results.extend(regular_results);
                Ok(final_results)
            }
            Err(e) => {
                search_log!(warn, "AI HTML analysis failed: {}, falling back to basic parsing", e);
                self.parse_generic_results(html)
            }
        }
    }

    /// 使用AI从HTML中提取种子信息
    async fn extract_torrents_from_html_with_ai(&self, html: &str, llm_client: Arc<dyn LlmClient>) -> Result<Vec<SearchResult>> {
        // 限制HTML长度以避免超出AI token限制 (250k tokens模型，使用80k字符约120k tokens)
        let truncated_html = if html.len() > 80000 {
            search_log!(info, "HTML too long ({} chars), truncating to 80k chars", html.len());
            safe_truncate(html, 80000)
        } else {
            html
        };

        // 直接传递原始HTML给AI服务，让llm_service.rs负责构建提示词
        match self.call_ai_for_html_analysis(truncated_html, llm_client).await {
            Ok(ai_results) => Ok(ai_results),
            Err(e) => Err(anyhow!("AI HTML analysis failed: {}", e))
        }
    }

    /// 直接调用AI进行HTML分析
    async fn call_ai_for_html_analysis(&self, html_content: &str, llm_client: Arc<dyn LlmClient>) -> Result<Vec<SearchResult>> {
        // 获取提取配置
        let extraction_config = self.extraction_config.as_ref()
            .ok_or_else(|| anyhow!("Extraction config not available"))?;

        // 将原始HTML传递给AI服务，由llm_service.rs构建提示词
        match llm_client.batch_extract_basic_info_from_html(html_content, extraction_config).await {
            Ok(batch_result) => {
                // AI返回的JSON响应被解析到batch_result.results中
                // 我们需要将整个结果传递给解析函数
                self.parse_ai_html_response_from_batch(batch_result)
            }
            Err(e) => {
                search_log!(error, "AI HTML分析失败: {}", e);
                search_log!(ai, "发送给AI的HTML长度: {} 字符", html_content.len());
                search_log!(ai, "HTML前500字符预览: {}", safe_truncate(html_content, 500));
                Err(anyhow!("AI HTML analysis failed: {}", e))
            }
        }
    }

    /// 解析AI返回的HTML分析结果
    fn parse_ai_html_response_from_batch(&self, batch_result: crate::llm_service::BatchExtractBasicInfoResult) -> Result<Vec<SearchResult>> {
        // 直接从BatchExtractBasicInfoResult转换为SearchResult
        let mut results = Vec::new();

        for basic_info in batch_result.results {
            // 验证磁力链接格式
            if !basic_info.magnet_link.starts_with("magnet:?xt=urn:btih:") {
                println!("⚠️ Invalid magnet link format, skipping: {}", basic_info.magnet_link);
                continue;
            }

            // 第一阶段AI只提取基础信息，文件列表需要根据标题生成
            let file_list = generate_file_list_from_title(&basic_info.title);

            // 处理 source_url：统一使用 normalize_source_url
            let source_url = basic_info
                .source_url
                .map(|href| self.normalize_source_url(&href));

            results.push(SearchResult {
                title: clean_html_text(&basic_info.title),
                magnet_link: basic_info.magnet_link,
                file_size: basic_info.file_size,
                upload_date: None, // 第一阶段不提取上传日期
                file_list,
                source_url,
                preview_image_url: None,
                score: None,
                tags: None,
            });
        }

        Ok(results)
    }

    /// 从URL模板中提取基础URL（用于构建完整的source_url）
    fn extract_base_url_from_template(&self) -> Option<String> {
        if let Ok(parsed_url) = url::Url::parse(&self.url_template) {
            if let Some(host) = parsed_url.host_str() {
                let scheme = parsed_url.scheme();
                return Some(format!("{scheme}://{host}"));
            }
        }
        None
    }

    /// 标准化source_url，将相对路径转换为绝对路径
    fn normalize_source_url(&self, href: &str) -> String {
        if href.starts_with("http://") || href.starts_with("https://") {
            href.to_string()
        } else if href.starts_with("//") {
            format!("https:{href}")
        } else if href.starts_with("/") {
            // 相对路径，需要从URL模板中提取基础域名
            self.extract_base_url_from_template()
                .map(|base| format!("{base}{href}"))
                .unwrap_or_else(|| href.to_string())
        } else {
            href.to_string()
        }
    }

    fn normalize_asset_url(&self, src: &str) -> String {
        if src.starts_with("http://") || src.starts_with("https://") {
            src.to_string()
        } else if src.starts_with("//") {
            format!("https:{src}")
        } else if src.starts_with("/") {
            self.extract_base_url_from_template()
                .map(|base| format!("{base}{src}"))
                .unwrap_or_else(|| src.to_string())
        } else if let Ok(template_url) = url::Url::parse(&self.url_template) {
            template_url
                .join(src)
                .map(|url| url.to_string())
                .unwrap_or_else(|_| src.to_string())
        } else {
            src.to_string()
        }
    }

    // 注意：parse_ai_html_response 函数已被删除，因为现在直接使用 BatchExtractBasicInfoResult

    /// 分离优先结果和普通结果
    fn separate_priority_results(&self, results: Vec<SearchResult>) -> (Vec<SearchResult>, Vec<SearchResult>) {
        if self.priority_keywords.is_empty() {
            return (Vec::new(), results);
        }

        let (priority_results, regular_results): (Vec<_>, Vec<_>) = results.into_iter().partition(|result| {
            let title_lower = result.title.to_lowercase();
            self.priority_keywords.iter().any(|keyword| title_lower.contains(&keyword.to_lowercase()))
        });

        if !priority_results.is_empty() {
            println!("🌟 Found {} priority results.", priority_results.len());
        }

        (priority_results, regular_results)
    }

    // 注意：apply_detailed_ai_analysis 方法已被移除
    // 现在统一使用前端的并行分析流程，提供更好的用户体验

    fn parse_generic_results(&self, html: &str) -> Result<Vec<SearchResult>> {
        let document = Html::parse_document(html);
        let mut results = Vec::new();
        let decoded_html = decode_html_entities(html);

        println!("🔍 Parsing generic HTML content...");

        // 尝试查找常见的磁力链接模式
        let magnet_regex = regex::Regex::new(r#"magnet:\?xt=urn:btih:[a-zA-Z0-9]{32,40}[^"'<>\s]*"#)
            .map_err(|e| anyhow!("Invalid regex: {}", e))?;

        // 尝试解析表格结构（最常见的种子站点布局）
        if let Ok(table_selector) = Selector::parse("table") {
            for table in document.select(&table_selector) {
                if let Ok(row_selector) = Selector::parse("tr") {
                    for row in table.select(&row_selector) {
                        if let Some(result) = self.parse_table_row(&row, &magnet_regex) {
                            results.push(result);
                        }
                    }
                }
            }
        }

        // 如果表格解析没有结果，尝试通用解析
        if results.is_empty() {
            results = self.parse_generic_fallback(&decoded_html, &magnet_regex)?;
        }

        println!("📊 Extracted {} unique results from generic HTML", results.len());
        Ok(results)
    }

    /// 解析表格行，提取标题、磁力链接和文件大小
    fn parse_table_row(&self, row: &scraper::ElementRef, magnet_regex: &regex::Regex) -> Option<SearchResult> {
        let row_html = row.html();
        let decoded_row_html = decode_html_entities(&row_html);

        // 查找磁力链接
        let magnet_link = if let Some(magnet_match) = magnet_regex.find(&decoded_row_html) {
            magnet_match.as_str().to_string()
        } else {
            self.extract_magnet_from_hash_link(row)?
        };

        // 提取单元格
        let cell_selector = Selector::parse("td").ok()?;
        let cells: Vec<_> = row.select(&cell_selector).collect();

        if cells.is_empty() {
            return None;
        }

        let mut title = None;
        let mut file_size = None;
        let mut upload_date = None;
        let mut source_url = None;

        // 分析每个单元格
        for (i, cell) in cells.iter().enumerate() {
            let cell_text = cell.text().collect::<String>().trim().to_string();

            // Most torrent tables put the title in an early cell, but not always the first one.
            if title.is_none() {
                if let Ok(link_selector) = Selector::parse("a") {
                    if let Some(link) = cell.select(&link_selector).next() {
                        let link_text = link.text().collect::<String>().trim().to_string();
                        if link_text.len() > 3 && !link_text.starts_with("magnet:") {
                            title = Some(clean_html_text(&link_text));
                            // 提取source_url
                            if let Some(href) = link.value().attr("href") {
                                source_url = Some(self.normalize_source_url(href));
                            }
                        }
                    }
                }
                // 如果没有链接，使用单元格文本
                if title.is_none() && i <= 2 && !cell_text.is_empty() && cell_text.len() > 5 {
                    title = Some(clean_html_text(&cell_text));
                }
            }

            // 查找文件大小（包含 GB, MB, KB, TB 的单元格）
            if file_size.is_none() && self.is_file_size(&cell_text) {
                file_size = Some(cell_text.clone());
            }

            // 查找日期（包含日期格式的单元格）
            if upload_date.is_none() && self.is_date(&cell_text) {
                upload_date = Some(cell_text);
            }
        }

        // 如果没有找到标题，尝试从磁力链接提取
        let final_title = title.unwrap_or_else(|| self.extract_title_from_magnet(&magnet_link));

        let file_list = generate_file_list_from_title(&final_title);

        Some(SearchResult {
            title: final_title,
            magnet_link,
            file_size,
            upload_date,
            file_list,
            source_url,
            preview_image_url: self.extract_preview_image(row),
            score: None,
            tags: None,
        })
    }

    /// 通用回退解析方法
    fn parse_generic_fallback(&self, decoded_html: &str, magnet_regex: &regex::Regex) -> Result<Vec<SearchResult>> {
        let mut results = Vec::new();
        let mut seen_magnets = HashSet::new();

        for magnet_match in magnet_regex.find_iter(decoded_html) {
            let magnet_link = magnet_match.as_str();

            if seen_magnets.insert(magnet_link.to_string()) {
                let title = self.extract_title_from_magnet(magnet_link);
                let file_list = generate_file_list_from_title(&title);

                results.push(SearchResult {
                    title,
                    magnet_link: magnet_link.to_string(),
                    file_size: None,
                    upload_date: None,
                    file_list,
                    source_url: None,
                    preview_image_url: None,
                    score: None,
                    tags: None,
                });
            }
        }

        Ok(results)
    }

    fn extract_magnet_from_hash_link(&self, row: &scraper::ElementRef) -> Option<String> {
        let link_selector = Selector::parse("a[href]").ok()?;
        let hash_regex = regex::Regex::new(r"(?i)([a-f0-9]{40})").ok()?;

        for link in row.select(&link_selector) {
            let href = link.value().attr("href")?;
            if let Some(capture) = hash_regex.captures(href) {
                let hash = capture.get(1)?.as_str().to_uppercase();
                let title = link.text().collect::<String>();
                let title = clean_html_text(&title);

                if title.is_empty() {
                    return Some(format!("magnet:?xt=urn:btih:{hash}"));
                }

                return Some(format!(
                    "magnet:?xt=urn:btih:{hash}&dn={}",
                    urlencoding::encode(&title)
                ));
            }
        }

        None
    }

    fn extract_preview_image(&self, element: &scraper::ElementRef) -> Option<String> {
        let image_selector = Selector::parse("img").ok()?;

        element
            .select(&image_selector)
            .filter_map(extract_image_source)
            .find(|src| is_content_preview_image(src))
            .map(|src| self.normalize_asset_url(src))
    }

    /// 判断文本是否是文件大小
    fn is_file_size(&self, text: &str) -> bool {
        let text_upper = text.to_uppercase();
        (text_upper.contains("GB") || text_upper.contains("MB") ||
         text_upper.contains("KB") || text_upper.contains("TB")) &&
        text.chars().any(|c| c.is_ascii_digit())
    }

    /// 判断文本是否是日期
    fn is_date(&self, text: &str) -> bool {
        // 简单的日期格式检测
        text.contains("-") && text.len() >= 8 && text.len() <= 20 &&
        text.chars().filter(|c| c.is_ascii_digit()).count() >= 4
    }

    /// 从磁力链接的dn参数中提取标题
    fn extract_title_from_magnet(&self, magnet_link: &str) -> String {
        // 尝试从磁力链接的dn参数中提取文件名
        if let Some(dn_start) = magnet_link.find("&dn=") {
            let dn_part = &magnet_link[dn_start + 4..];
            if let Some(dn_end) = dn_part.find('&') {
                let dn_value = &dn_part[..dn_end];
                // URL解码
                if let Ok(decoded) = urlencoding::decode(dn_value) {
                    let decoded_str = decoded.to_string();
                    if !decoded_str.is_empty() && decoded_str.len() > 5 {
                        return decoded_str;
                    }
                }
            } else {
                // dn是最后一个参数
                if let Ok(decoded) = urlencoding::decode(dn_part) {
                    let decoded_str = decoded.to_string();
                    if !decoded_str.is_empty() && decoded_str.len() > 5 {
                        return decoded_str;
                    }
                }
            }
        }

        // 如果无法从dn参数提取，生成一个基于哈希的标题
        let hash_part = if let Some(btih_start) = magnet_link.find("btih:") {
            let hash_start = btih_start + 5;
            let hash_part = &magnet_link[hash_start..];
            if let Some(hash_end) = hash_part.find('&') {
                &hash_part[..hash_end.min(8)]
            } else {
                &hash_part[..8.min(hash_part.len())]
            }
        } else {
            "unknown"
        };

        format!("Torrent_{hash_part}")
    }
}

/// 根据标题生成相关的文件列表
fn generate_file_list_from_title(title: &str) -> Vec<String> {
    let mut file_list = Vec::new();
    let title_lower = title.to_lowercase();

    // 根据标题内容生成相关的文件列表
    if title_lower.contains("电影") || title_lower.contains("movie") || title_lower.contains("film") {
        // 电影类型
        let base_name = extract_clean_title(title);
        file_list.push(format!("{base_name}.1080p.BluRay.x264.mkv"));
        file_list.push(format!("{base_name}.720p.BluRay.x264.mkv"));
        file_list.push("Subtitles/Chinese.srt".to_string());
        file_list.push("Subtitles/English.srt".to_string());
        file_list.push("Sample.mkv".to_string());
    } else if title_lower.contains("s0") || title_lower.contains("season") || title_lower.contains("集") {
        // 电视剧类型
        let base_name = extract_clean_title(title);
        for i in 1..=10 {
            file_list.push(format!("{base_name}.S01E{i:02}.1080p.WEB-DL.x264.mkv"));
        }
        file_list.push("Subtitles/Chinese.srt".to_string());
        file_list.push("Subtitles/English.srt".to_string());
    } else if title_lower.contains("游戏") || title_lower.contains("game") {
        // 游戏类型
        let base_name = extract_clean_title(title);
        file_list.push(format!("{base_name}.exe"));
        file_list.push("Setup.exe".to_string());
        file_list.push("Crack/Keygen.exe".to_string());
        file_list.push("README.txt".to_string());
    } else if title_lower.contains("音乐") || title_lower.contains("music") || title_lower.contains("mp3") || title_lower.contains("flac") {
        // 音乐类型
        let base_name = extract_clean_title(title);
        for i in 1..=12 {
            file_list.push(format!("{base_name} - Track {i:02}.mp3"));
        }
        file_list.push("Cover.jpg".to_string());
    } else if title_lower.contains("软件") || title_lower.contains("software") || title_lower.contains("app") {
        // 软件类型
        let base_name = extract_clean_title(title);
        file_list.push(format!("{base_name}_Setup.exe"));
        file_list.push("Crack/Patch.exe".to_string());
        file_list.push("License.txt".to_string());
        file_list.push("README.txt".to_string());
    } else {
        // 默认类型 - 基于标题生成通用文件
        let base_name = extract_clean_title(title);
        file_list.push(format!("{base_name}.mkv"));
        file_list.push(format!("{base_name}.mp4"));
        file_list.push("README.txt".to_string());
    }

    // 添加一些通用文件
    if !file_list.iter().any(|f| f.contains("README")) {
        file_list.push("README.txt".to_string());
    }

    file_list
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;

    let bytes = bytes as f64;
    if bytes >= TB {
        format!("{:.2} TB", bytes / TB)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes / KB)
    } else {
        format!("{bytes:.0} B")
    }
}

fn format_unix_date(timestamp: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|datetime| datetime.date_naive().to_string())
}

fn is_content_preview_image(src: &str) -> bool {
    let src_lower = src.to_lowercase();
    !src_lower.starts_with("data:")
        && !src_lower.contains("logo")
        && !src_lower.contains("icon")
        && !src_lower.contains("sprite")
        && !src_lower.contains("avatar")
        && !src_lower.contains("loading")
        && !src_lower.contains("blank")
        && !src_lower.ends_with(".svg")
}

fn extract_image_source(image: scraper::ElementRef) -> Option<&str> {
    ["data-original", "data-src", "file", "zoomfile", "src"]
        .iter()
        .filter_map(|attr| image.value().attr(attr))
        .find(|src| !src.trim().is_empty())
}

/// 从标题中提取干净的名称（移除特殊字符和格式信息）
/// 用途：用于搜索解析阶段生成稳定的文件名，尽量保证可预测与无特殊字符。
/// 注意：展示给用户的标题清理应使用 `clean_title_unified`（main.rs）。
fn extract_clean_title(title: &str) -> String {
    let mut clean_title = title.to_string();

    // 移除常见的格式标识
    let patterns_to_remove = [
        r"\[.*?\]", r"\(.*?\)", r"【.*?】", r"（.*?）",
        r"1080p", r"720p", r"4K", r"BluRay", r"WEB-DL", r"HDTV",
        r"x264", r"x265", r"H\.264", r"H\.265", r"HEVC",
        r"DTS", r"AC3", r"AAC", r"MP3", r"FLAC",
        r"mkv", r"mp4", r"avi", r"rmvb", r"wmv"
    ];

    for pattern in &patterns_to_remove {
        if let Ok(re) = regex::Regex::new(&format!("(?i){pattern}")) {
            clean_title = re.replace_all(&clean_title, "").to_string();
        }
    }

    // 清理多余的空格和特殊字符
    clean_title = clean_title
        .trim()
        .replace("  ", " ")
        .replace(" ", "_")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();

    if clean_title.is_empty() {
        "Unknown".to_string()
    } else {
        clean_title
    }
}

/// 搜索引擎核心
pub struct SearchCore {
    providers: Vec<Arc<dyn SearchProvider>>,
}

impl SearchCore {
    // 注意：基础构造函数已被删除，统一使用 create_ai_enhanced_search_core

    /// 多页搜索 - 按提供商顺序搜索，优先返回clmclm结果
    pub async fn search_multi_page(&self, query: &str, max_pages: u32) -> Result<Vec<SearchResult>> {
        if self.providers.is_empty() {
            return Err(anyhow!("No search providers available"));
        }

        println!("🔍 Starting search with {} providers, {} pages each", self.providers.len(), max_pages);

        let mut all_results = Vec::new();

        // 分离clmclm和其他提供商
        let mut clmclm_provider = None;
        let mut other_providers = Vec::new();

        for provider in &self.providers {
            if provider.name() == "clmclm.com" {
                clmclm_provider = Some(Arc::clone(provider));
            } else {
                other_providers.push(Arc::clone(provider));
            }
        }

        // 1. 首先搜索clmclm（如果启用）
        if let Some(clmclm) = clmclm_provider {
            println!("🔍 Searching clmclm.com first for faster results");
            for page in 1..=max_pages {
                match clmclm.search(query, page).await {
                    Ok(mut results) => {
                        let count = results.len();
                        println!("✅ clmclm.com page {page} returned {count} results");
                        all_results.append(&mut results);
                    }
                    Err(e) => {
                        println!("❌ clmclm.com page {page} failed: {e}");
                    }
                }
            }
        }

        // 2. 然后并发搜索其他提供商
        if !other_providers.is_empty() {
            println!("🔍 Now searching {} other providers concurrently", other_providers.len());

            let mut other_search_futures = Vec::new();

            for provider in other_providers {
                for page in 1..=max_pages {
                    let provider = Arc::clone(&provider);
                    let query = query.to_string();
                    let provider_name = provider.name().to_string();

                    let search_future = async move {
                        println!("🔍 Searching {query} page {page} with provider: {provider_name}");
                        match provider.search(&query, page).await {
                            Ok(results) => {
                                let count = results.len();
                                println!("✅ Provider {provider_name} page {page} returned {count} results");
                                Ok(results)
                            }
                            Err(e) => {
                                println!("❌ Provider {provider_name} page {page} failed: {e}");
                                Err(e)
                            }
                        }
                    };

                    other_search_futures.push(search_future);
                }
            }

            // 并发执行其他搜索任务
            let results = join_all(other_search_futures).await;

            for result in results {
                match result {
                    Ok(mut page_results) => {
                        all_results.append(&mut page_results);
                    }
                    Err(e) => {
                        println!("⚠️ Search task failed: {e}");
                        // 继续处理其他结果，不因为单个任务失败而中断
                    }
                }
            }
        }

        let original_count = all_results.len();
        let mut seen_magnets = HashSet::new();
        all_results.retain(|result| seen_magnets.insert(result.magnet_link.clone()));

        if all_results.len() != original_count {
            println!(
                "🧹 Removed {} duplicate results",
                original_count - all_results.len()
            );
        }

        println!("🎯 Total results collected from all providers: {}", all_results.len());
        Ok(all_results)
    }



    /// 单页搜索（向后兼容）
    #[allow(dead_code)]
    pub async fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        self.search_multi_page(query, 1).await
    }
}

/// 创建带有AI功能的搜索核心
pub fn create_ai_enhanced_search_core(
    extraction_config: Option<LlmConfig>,
    analysis_config: Option<LlmConfig>, // 保持向后兼容，但现在只用于HTML提取
    priority_keywords: Vec<String>,
    custom_engines: Vec<(String, String)>, // (name, url_template) pairs
    include_clmclm: bool // 是否包含 clmclm.com
) -> SearchCore {
    let mut providers: Vec<Arc<dyn SearchProvider>> = Vec::new();

    // 只有在明确启用时才添加 clmclm.com 提供商
    if include_clmclm {
        println!("✅ Adding clmclm.com provider");
        providers.push(Arc::new(ClmclmProvider::new()));
    }

    // 为自定义搜索引擎创建AI增强的提供商
    // 优先使用 extraction_config，如果没有则使用 analysis_config（向后兼容）
    let html_extraction_config = extraction_config.or(analysis_config);

    if let Some(extract_config) = html_extraction_config {
        let llm_client: Arc<dyn LlmClient> = Arc::new(GeminiClient::new());

        for (name, url_template) in custom_engines {
            println!("✅ Adding AI-enhanced custom provider: {name}");
            let provider = GenericProvider::new(name, url_template)
                .with_llm_client_and_config(llm_client.clone(), extract_config.clone())
                .with_priority_keywords(priority_keywords.clone());
            providers.push(Arc::new(provider));
        }
    } else {
        // 如果没有LLM配置，创建基础的自定义提供商
        for (name, url_template) in custom_engines {
            println!("✅ Adding basic custom provider: {name}");
            let provider = GenericProvider::new(name, url_template);
            providers.push(Arc::new(provider));
        }
    }

    SearchCore { providers }
}



#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    // removed redundant single-component import per clippy

    #[tokio::test]
    async fn test_search_successful() {
        // Start a mock server
        let server = MockServer::start();

        // Create a mock
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/search-test-1-1-1.html");
            then.status(200)
                .header("content-type", "text/html; charset=UTF-8")
                .body(r#"
                    <!DOCTYPE html>
                    <html>
                    <body>
                        <div class="ssbox">
                            <div class="title"><h3><a href="/detail/123">Test Title 1</a></h3></div>
                            <div class="sbar">
                                <a href="magnet:?xt=urn:btih:12345">Magnet Link</a>
                                <span>大小: 1.2GB</span>
                            </div>
                            <ul>
                                <li>File A 700MB</li>
                                <li>File B 500MB</li>
                            </ul>
                        </div>
                        <div class="ssbox">
                            <div class="title"><h3><a href="/detail/678">Test Title 2</a></h3></div>
                            <div class="sbar">
                                <a href="magnet:?xt=urn:btih:67890">Magnet Link</a>
                                <span>大小: 900MB</span>
                            </div>
                            <ul>
                                <li>Episode 01 450MB</li>
                                <li>Episode 02 450MB</li>
                            </ul>
                        </div>
                    </body>
                    </html>
                "#);
        });

        // Perform the search against the mock server
        let provider = ClmclmProvider::with_base_url(&server.base_url());
        let results = provider.search("test", 1).await.unwrap();

        // Assert
        mock.assert();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Test Title 1");
        assert_eq!(results[0].magnet_link, "magnet:?xt=urn:btih:12345");
        assert_eq!(results[1].title, "Test Title 2");
        assert_eq!(results[1].magnet_link, "magnet:?xt=urn:btih:67890");
    }

    #[tokio::test]
    async fn test_search_no_results() {
        // Start a mock server
        let server = MockServer::start();

        // Create a mock for a page with no items
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/search-empty-1-1-1.html");
            then.status(200)
                .header("content-type", "text/html; charset=UTF-8")
                .body(r#"
                    <!DOCTYPE html>
                    <html>
                    <body>
                        <p>No results found.</p>
                    </body>
                    </html>
                "#);
        });

        // Perform the search
        let provider = ClmclmProvider::with_base_url(&server.base_url());
        let results = provider.search("empty", 1).await.unwrap();

        // Assert
        mock.assert();
        assert!(results.is_empty());
    }

    #[test]
    fn test_generic_parser_handles_encoded_magnet_links() {
        let provider = GenericProvider::new(
            "Bitsearch".to_string(),
            "https://example.test/search?q={keyword}&page={page}".to_string(),
        );
        let html = r#"
            <html>
            <body>
                <a href="magnet:?xt&#x3D;urn:btih:D540FC48EB12F2833163EED6421D449DD8F1CE1F&amp;dn&#x3D;Ubuntu">
                    Magnet
                </a>
            </body>
            </html>
        "#;

        let results = provider.parse_generic_results(html).unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].magnet_link.starts_with("magnet:?xt=urn:btih:D540FC48EB12F2833163EED6421D449DD8F1CE1F"));
    }

    #[test]
    fn test_torrents_csv_json_parser_builds_magnet_links() {
        let provider = GenericProvider::new(
            "TorrentsCSV".to_string(),
            "https://torrents-csv.com/service/search?q={keyword}&size=50".to_string(),
        );
        let json = r#"{
            "torrents": [
                {
                    "infohash": "29f629d0586efe2f2327ecd7dbc63797437aacde",
                    "name": "Ubuntu Test ISO",
                    "size_bytes": 1112120097,
                    "created_unix": 1531492876
                }
            ]
        }"#;

        let results = provider.parse_json_results(json).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Ubuntu Test ISO");
        assert!(results[0].magnet_link.starts_with("magnet:?xt=urn:btih:29f629d0586efe2f2327ecd7dbc63797437aacde"));
        assert_eq!(results[0].file_size.as_deref(), Some("1.04 GB"));
    }
}
