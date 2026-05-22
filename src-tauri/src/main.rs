#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// 引入我们的新模块
mod llm_service;
use crate::llm_service::LlmClient;
// 引入需要的模块
mod searcher;
mod app_state;
mod i18n;

use tauri::Manager;
use regex::Regex;
use searcher::SearchCore;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

// ============ 辅助函数 ============

/// 从 AppState 构建 LLM 配置
fn build_llm_configs(app_state: &app_state::AppState) -> (Option<llm_service::LlmConfig>, Option<llm_service::LlmConfig>) {
    let llm_config = app_state::get_llm_config(app_state);

    let extraction_config = if !llm_config.extraction_config.api_key.is_empty() {
        Some(llm_service::LlmConfig {
            provider: llm_config.extraction_config.provider.clone(),
            api_key: llm_config.extraction_config.api_key.clone(),
            api_base: llm_config.extraction_config.api_base.clone(),
            model: llm_config.extraction_config.model.clone(),
            batch_size: llm_config.extraction_config.batch_size,
        })
    } else {
        None
    };

    let analysis_config = if !llm_config.analysis_config.api_key.is_empty() {
        Some(llm_service::LlmConfig {
            provider: llm_config.analysis_config.provider.clone(),
            api_key: llm_config.analysis_config.api_key.clone(),
            api_base: llm_config.analysis_config.api_base.clone(),
            model: llm_config.analysis_config.model.clone(),
            batch_size: llm_config.analysis_config.batch_size,
        })
    } else {
        None
    };

    (extraction_config, analysis_config)
}

/// 从 AppState 获取启用的搜索引擎
fn get_active_engines(app_state: &app_state::AppState) -> Vec<app_state::SearchEngine> {
    app_state::get_all_engines(app_state)
        .into_iter()
        .filter(|e| e.is_enabled)
        .collect()
}

/// 从 AppState 获取优先关键词
fn get_priority_keywords(app_state: &app_state::AppState) -> Vec<String> {
    app_state::get_all_priority_keywords(app_state)
        .iter()
        .map(|pk| pk.keyword.clone())
        .collect()
}

/// 创建 SearchCore 实例
fn create_search_core(
    state: &app_state::AppState,
    include_clmclm: bool,
    include_others: bool,
) -> Result<SearchCore, String> {
    let (extraction_config, analysis_config) = build_llm_configs(state);
    let priority_keyword_strings = get_priority_keywords(state);
    let enabled_engines = get_active_engines(state);

    let clmclm_is_enabled_in_settings = enabled_engines.iter().any(|e| e.name == "clmclm.com");

    let custom_engine_tuples: Vec<(String, String)> = if include_others {
        enabled_engines
            .iter()
            .filter(|e| e.name != "clmclm.com")
            .map(|e| (e.name.clone(), e.url_template.clone()))
            .collect()
    } else {
        Vec::new()
    };

    let final_clmclm_status = include_clmclm && clmclm_is_enabled_in_settings;

    if custom_engine_tuples.is_empty() && !final_clmclm_status {
        return Err(i18n::translate_error(&i18n::ErrorCode::SearchNoEngines));
    }

    println!(
        "🔧 Creating search core: Custom Engines: {}, CLMCLM: {}",
        custom_engine_tuples.len(),
        final_clmclm_status
    );

    Ok(searcher::create_ai_enhanced_search_core(
        extraction_config,
        analysis_config,
        priority_keyword_strings,
        custom_engine_tuples,
        final_clmclm_status,
    ))
}

// ============ AI分析命令 ============

/// 统一的标题清理函数
/// 用途：在第二阶段（分析后）回填标题时，做最少量的清理，保持人类可读性。
/// 注意：搜索阶段的文件名生成应使用 `extract_clean_title`（searcher.rs）以保证可预期的文件名格式。
fn clean_title_unified(title: &str) -> String {
    if title.trim().is_empty() {
        return "Unknown".to_string();
    }

    // 移除常见的广告标记，如 [y5y4.com] 或 【...】
    let re_brackets = Regex::new(r"\[.*?\]|【.*?】").unwrap();
    let title = re_brackets.replace_all(title, "");

    // 移除常见的URL和推广信息
    let re_urls = Regex::new(r"(?i)(www\.\S+\.\S+|https?://\S+)").unwrap();
    let title = re_urls.replace_all(&title, "");

    // 清理多余的空格
    let cleaned = title.trim().replace("  ", " ");

    if cleaned.is_empty() {
        "Unknown".to_string()
    } else {
        cleaned
    }
}

/// 创建DetailedAnalysisResult的辅助函数
fn create_analysis_result(
    original_result: &searcher::SearchResult,
    cleaned_title: Option<String>,
    purity_score: u8,
    tags: Vec<String>,
    error: Option<String>,
) -> llm_service::DetailedAnalysisResult {
    let final_title = cleaned_title.unwrap_or_else(|| clean_title_unified(&original_result.title));

    llm_service::DetailedAnalysisResult {
        title: final_title,
        purity_score,
        tags,
        magnet_link: original_result.magnet_link.clone(),
        file_size: original_result.file_size.clone(),
        file_list: original_result.file_list.clone(),
        error,
    }
}


#[tauri::command]
async fn analyze_resource(
    result: searcher::SearchResult,
    llm_config: llm_service::LlmConfig,
) -> Result<llm_service::DetailedAnalysisResult, String> {
    let client = llm_service::GeminiClient::new();

    match client.batch_analyze_scores_and_tags(&result.title, &result.file_list, &llm_config).await {
        Ok((cleaned_title, score, tags)) => {
            // 简化调试输出
            println!("[AI] Analyzed: '{}' -> '{}'", result.title, cleaned_title);

            let final_title = if cleaned_title.is_empty() {
                clean_title_unified(&result.title)
            } else {
                cleaned_title
            };

            Ok(llm_service::DetailedAnalysisResult {
                title: final_title,
                purity_score: score,
                tags,
                magnet_link: result.magnet_link,
                file_size: result.file_size,
                file_list: result.file_list,
                error: None,
            })
        }
        Err(e) => Err(e.to_string()),
    }
}


// ============ 收藏夹相关命令 ============

#[tauri::command]
async fn add_to_favorites(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    title: String,
    magnet_link: String,
    file_size: Option<String>,
    file_list: Vec<String>,
) -> Result<app_state::FavoriteItem, String> {
    let result = app_state::add_to_favorites(&state, title, magnet_link, file_size, file_list)
        .map_err(|e| e.to_string())?;

    // 保存状态到文件
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    Ok(result)
}

#[tauri::command]
async fn get_all_favorites(state: tauri::State<'_, app_state::AppState>) -> Result<Vec<app_state::FavoriteItem>, String> {
    Ok(app_state::get_all_favorites(&state))
}

#[tauri::command]
async fn remove_from_favorites(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    id: String,
) -> Result<(), String> {
    app_state::remove_from_favorites(&state, id).map_err(|e| e.to_string())?;

    // 保存状态到文件
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn search_favorites(
    state: tauri::State<'_, app_state::AppState>,
    query: String,
) -> Result<Vec<app_state::FavoriteItem>, String> {
    Ok(app_state::search_favorites(&state, query))
}



#[tauri::command]
async fn search_multi_page(
    state: tauri::State<'_, app_state::AppState>,
    keyword: String,
    max_pages: Option<u32>,
) -> Result<Vec<searcher::SearchResult>, String> {
    let pages = max_pages.unwrap_or(3);
    let search_core = create_search_core(&state, true, true)?;
    search_core.search_multi_page(keyword.as_str(), pages).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn search_clmclm_first(
    state: tauri::State<'_, app_state::AppState>,
    keyword: String,
    max_pages: Option<u32>,
) -> Result<Vec<searcher::SearchResult>, String> {
    let pages = max_pages.unwrap_or(3);
    match create_search_core(&state, true, false) {
        Ok(search_core) => search_core.search_multi_page(keyword.as_str(), pages).await.map_err(|e| e.to_string()),
        Err(_) => Ok(Vec::new()), // 如果clmclm未启用，则返回空结果
    }
}

#[tauri::command]
async fn search_other_engines(
    state: tauri::State<'_, app_state::AppState>,
    keyword: String,
    max_pages: Option<u32>,
) -> Result<Vec<searcher::SearchResult>, String> {
    let pages = max_pages.unwrap_or(3);
    match create_search_core(&state, false, true) {
        Ok(search_core) => search_core.search_multi_page(keyword.as_str(), pages).await.map_err(|e| e.to_string()),
        Err(_) => Ok(Vec::new()), // 如果没有其他引擎，则返回空结果
    }
}



// ============ 搜索引擎相关命令 ============

#[tauri::command]
async fn add_search_engine(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    name: String,
    url_template: String,
) -> Result<app_state::SearchEngine, String> {
    let result = app_state::add_search_engine(&state, name, url_template)
        .map_err(|e| e.to_string())?;

    // 保存状态到文件
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    Ok(result)
}

#[tauri::command]
async fn update_search_engine(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    id: String,
    name: String,
    url_template: String,
) -> Result<(), String> {
    app_state::update_search_engine(&state, id, name, url_template)
        .map_err(|e| e.to_string())?;

    // 保存状态到文件
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn get_all_engines(state: tauri::State<'_, app_state::AppState>) -> Result<Vec<app_state::SearchEngine>, String> {
    Ok(app_state::get_all_engines(&state))
}

#[tauri::command]
async fn update_engine_status(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    id: String,
    is_enabled: bool,
) -> Result<(), String> {
    app_state::update_engine_status(&state, id, is_enabled).map_err(|e| e.to_string())?;

    // 保存状态到文件
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn delete_engine(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    id: String,
) -> Result<(), String> {
    app_state::delete_engine(&state, id).map_err(|e| e.to_string())?;

    // 保存状态到文件
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    Ok(())
}

// ============ 优先关键词相关命令 ============

#[tauri::command]
async fn add_priority_keyword(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    keyword: String,
) -> Result<app_state::PriorityKeyword, String> {
    let result = app_state::add_priority_keyword(&state, keyword)
        .map_err(|e| e.to_string())?;

    // 保存状态到文件
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    Ok(result)
}

#[tauri::command]
async fn get_all_priority_keywords(state: tauri::State<'_, app_state::AppState>) -> Result<Vec<app_state::PriorityKeyword>, String> {
    Ok(app_state::get_all_priority_keywords(&state))
}

#[tauri::command]
async fn delete_priority_keyword(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    id: String,
) -> Result<(), String> {
    app_state::delete_priority_keyword(&state, id).map_err(|e| e.to_string())?;

    // 保存状态到文件
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn test_connection(config: llm_service::LlmConfig) -> Result<String, String> {
    llm_service::test_connection(&config).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn test_extraction_connection(config: app_state::SingleLlmConfig) -> Result<String, String> {
    let llm_config = llm_service::LlmConfig {
        provider: config.provider,
        api_key: config.api_key,
        api_base: config.api_base,
        model: config.model,
        batch_size: config.batch_size,
    };
    llm_service::test_connection(&llm_config).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn test_analysis_connection(config: app_state::SingleLlmConfig) -> Result<String, String> {
    let llm_config = llm_service::LlmConfig {
        provider: config.provider,
        api_key: config.api_key,
        api_base: config.api_base,
        model: config.model,
        batch_size: config.batch_size,
    };
    llm_service::test_connection(&llm_config).await.map_err(|e| e.to_string())
}

// 注意：load_llm_config_from_app 和 load_llm_config_from_file 函数已被删除
// 因为它们未被使用，LLM配置现在通过前端直接传递

// ============ LLM 配置相关命令 ============

#[tauri::command]
async fn get_llm_config(state: tauri::State<'_, app_state::AppState>) -> Result<app_state::LlmConfig, String> {
    let config = app_state::get_llm_config(&state);
    println!("🔧 Get LLM config: extraction_batch_size={}, analysis_batch_size={}", config.extraction_config.batch_size, config.analysis_config.batch_size);
    Ok(config)
}



#[tauri::command]
async fn batch_analyze_resources(
    state: tauri::State<'_, app_state::AppState>,
    results: Vec<searcher::SearchResult>,
) -> Result<Vec<llm_service::DetailedAnalysisResult>, String> {
    let config = app_state::get_llm_config(&state);

    println!("🔧 Frontend batch analysis: {} results, batch_size={}", results.len(), config.analysis_config.batch_size);

    if results.is_empty() {
        return Ok(Vec::new());
    }

    // 转换为批量分析格式
    let batch_items: Vec<llm_service::BatchAnalysisItem> = results
        .iter()
        .filter(|r| !r.file_list.is_empty())
        .map(|r| llm_service::BatchAnalysisItem {
            title: r.title.clone(),
            file_list: r.file_list.clone(),
        })
        .collect();

    if batch_items.is_empty() {
        println!("⚠️ No valid results with file lists for batch analysis");
        return Ok(Vec::new());
    }

    // 转换配置
    let llm_config = llm_service::LlmConfig {
        provider: config.analysis_config.provider,
        api_key: config.analysis_config.api_key,
        api_base: config.analysis_config.api_base,
        model: config.analysis_config.model,
        batch_size: config.analysis_config.batch_size,
    };

    let client = llm_service::GeminiClient::new();
    let batch_size = config.analysis_config.batch_size as usize;
    let mut all_results = Vec::new();
    let mut failed_batches = 0;
    const MAX_FAILED_BATCHES: usize = 3; // 最多允许3个批次失败

    // 分批处理
    for (batch_index, chunk) in batch_items.chunks(batch_size).enumerate() {
        use std::num::NonZeroUsize;
        let Some(nz_batch) = NonZeroUsize::new(batch_size) else { continue };
        println!(
            "🔄 Frontend processing batch {}/{} ({} items)",
            batch_index + 1,
            batch_items.len().div_ceil(nz_batch.get()),
            chunk.len()
        );

        // 如果失败的批次太多，直接返回错误
        if failed_batches >= MAX_FAILED_BATCHES {
            return Err(format!("Too many batch failures ({failed_batches}/{MAX_FAILED_BATCHES}), aborting analysis"));
        }

        match client.batch_analyze_multiple_items(chunk, &llm_config).await {
            Ok(batch_results) => {
                // 将批量结果转换为 DetailedAnalysisResult
                for (i, analysis_result) in batch_results.iter().enumerate() {
                    if let Some(original_result) = results.get(batch_index * batch_size + i) {
                        let cleaned_title = if analysis_result.cleaned_title.is_empty() {
                            None
                        } else {
                            Some(analysis_result.cleaned_title.clone())
                        };

                        all_results.push(create_analysis_result(
                            original_result,
                            cleaned_title,
                            analysis_result.purity_score,
                            analysis_result.tags.clone(),
                            None,
                        ));
                    }
                }
                println!("✅ Frontend batch {} success.", batch_index + 1);
            }
            Err(e) => {
                failed_batches += 1;
                println!("⚠️ Frontend batch {} failed ({}/{}): {}", batch_index + 1, failed_batches, MAX_FAILED_BATCHES, e);

                // 如果这是最后一次尝试，直接添加失败结果而不进行单个分析
                if failed_batches >= MAX_FAILED_BATCHES {
                    for (i, _item) in chunk.iter().enumerate() {
                        if let Some(original_result) = results.get(batch_index * batch_size + i) {
                            all_results.push(create_analysis_result(
                                original_result,
                                None,
                                50, // 默认分数
                                vec!["Analysis Failed - Too Many Failures".to_string()],
                                Some("Too many batch failures, analysis aborted".to_string()),
                            ));
                        }
                    }
                    continue;
                }

                // 回退到单个分析（使用批量分析处理单个项目）
                for (i, item) in chunk.iter().enumerate() {
                    if let Some(original_result) = results.get(batch_index * batch_size + i) {
                        // 将单个项目包装为批量格式
                        let single_item = vec![item.clone()];

                        // 单个分析只尝试一次，不进行重试
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(30), // 30秒超时
                            client.batch_analyze_multiple_items(&single_item, &llm_config)
                        ).await {
                            Ok(Ok(mut batch_results)) => {
                                if let Some(result) = batch_results.pop() {
                                    let cleaned_title = if result.cleaned_title.is_empty() {
                                        None
                                    } else {
                                        Some(result.cleaned_title)
                                    };

                                    all_results.push(create_analysis_result(
                                        original_result,
                                        cleaned_title,
                                        result.purity_score,
                                        result.tags,
                                        None,
                                    ));
                                } else {
                                    println!("⚠️ Individual analysis for '{}' returned no results", item.title);
                                    all_results.push(create_analysis_result(
                                        original_result,
                                        None,
                                        50,
                                        vec!["No Results".to_string()],
                                        Some("Individual analysis returned no results".to_string()),
                                    ));
                                }
                            }
                            Ok(Err(individual_error)) => {
                println!("⚠️ Individual analysis for '{}' failed: {}", item.title, individual_error);
                                all_results.push(create_analysis_result(
                                    original_result,
                                    None,
                                    50,
                    vec!["Individual Analysis Failed".to_string()],
                    Some(format!("Individual analysis failed: {individual_error}")),
                                ));
                            }
                            Err(_timeout) => {
                                println!("⚠️ Individual analysis for '{}' timed out", item.title);
                                all_results.push(create_analysis_result(
                                    original_result,
                                    None,
                                    50,
                                    vec!["Analysis Timeout".to_string()],
                                    Some("Analysis timed out after 30 seconds".to_string()),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    println!("🎉 Frontend batch analysis completed: {} results processed", all_results.len());
    Ok(all_results)
}

#[tauri::command]
async fn update_llm_config(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    config: app_state::LlmConfig,
) -> Result<(), String> {
    println!("🔧 Updating LLM config: extraction_batch_size={}, analysis_batch_size={}", config.extraction_config.batch_size, config.analysis_config.batch_size);

    app_state::update_llm_config(&state, config).map_err(|e| e.to_string())?;

    // 保存状态到文件
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    println!("🔧 LLM config saved.");
    Ok(())
}

// ============ 搜索设置相关命令 ============

#[tauri::command]
async fn get_search_settings(state: tauri::State<'_, app_state::AppState>) -> Result<app_state::SearchSettings, String> {
    Ok(app_state::get_search_settings(&state))
}

#[tauri::command]
async fn update_search_settings(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    settings: app_state::SearchSettings,
) -> Result<(), String> {
    app_state::update_search_settings(&state, settings).map_err(|e| e.to_string())?;

    // 保存状态到文件
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    Ok(())
}

// ============ 下载配置相关命令 ============

#[tauri::command]
async fn get_download_config(state: tauri::State<'_, app_state::AppState>) -> Result<app_state::DownloadConfig, String> {
    Ok(app_state::get_download_config(&state))
}

#[tauri::command]
async fn update_download_config(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    config: app_state::DownloadConfig,
) -> Result<(), String> {
    app_state::update_download_config(&state, config).map_err(|e| e.to_string())?;

    // 保存状态到文件
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn refresh_tracker_servers(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
) -> Result<app_state::DownloadConfig, String> {
    let config = app_state::get_download_config(&state);
    let trackers = fetch_tracker_servers(&config.tracker_sources).await?;

    app_state::update_tracker_servers(&state, trackers, chrono::Utc::now().to_rfc3339())
        .map_err(|e| e.to_string())?;
    app_state::save_app_state(&app_handle, &state).map_err(|e| e.to_string())?;

    Ok(app_state::get_download_config(&state))
}

#[tauri::command]
async fn open_magnet_link(
    state: tauri::State<'_, app_state::AppState>,
    magnet_link: String,
) -> Result<(), String> {
    let config = app_state::get_download_config(&state);

    if !magnet_link.starts_with("magnet:?") {
        return Err("Invalid magnet link.".to_string());
    }

    if let Some(ref app_path) = config.custom_app_path {
        // 检查是否是115浏览器
        if app_path.to_lowercase().contains("115chrome") || app_path.to_lowercase().contains("115browser") {
            // 为115浏览器创建临时HTML文件
            create_and_open_magnet_html(&magnet_link, app_path, &config).await?;
        } else {
            open_magnet_with_app(&magnet_link, app_path)?;
        }
    } else {
        open_magnet_with_default_app(&magnet_link)?;
    }

    Ok(())
}

#[tauri::command]
async fn play_magnet_link(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    magnet_link: String,
) -> Result<(), String> {
    if !magnet_link.starts_with("magnet:?") {
        return Err("Invalid magnet link.".to_string());
    }

    let config = ensure_daily_tracker_update(&app_handle, &state).await?;
    let enhanced_magnet = append_trackers_to_magnet(&magnet_link, &config.tracker_servers);

    if let Some(ref app_path) = config.custom_app_path {
        open_magnet_with_app(&enhanced_magnet, app_path)?;
    } else {
        open_magnet_with_default_app(&enhanced_magnet)?;
    }

    Ok(())
}

#[derive(serde::Serialize)]
struct BuiltinPlayerSession {
    base_url: String,
    info_hash: String,
}

#[derive(serde::Serialize)]
struct BuiltinPlayerFile {
    name: String,
    url: String,
    length: Option<u64>,
    media_type: String,
}

#[derive(serde::Serialize)]
struct BuiltinPlayerStats {
    progress: Option<f64>,
    download_speed: Option<u64>,
    peers: Option<u64>,
}

#[tauri::command]
async fn start_builtin_magnet_player(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    magnet_link: String,
) -> Result<BuiltinPlayerSession, String> {
    if !magnet_link.starts_with("magnet:?") {
        return Err("Invalid magnet link.".to_string());
    }

    let info_hash = extract_magnet_info_hash(&magnet_link)?;
    let config = ensure_daily_tracker_update(&app_handle, &state).await?;
    let enhanced_magnet = append_trackers_to_magnet(&magnet_link, &config.tracker_servers);

    ensure_rqbit_server(&app_handle).await?;
    add_magnet_to_rqbit(&enhanced_magnet).await?;

    Ok(BuiltinPlayerSession {
        base_url: rqbit_base_url(),
        info_hash,
    })
}

#[tauri::command]
async fn get_builtin_magnet_playlist(info_hash: String) -> Result<Vec<BuiltinPlayerFile>, String> {
    let playlist_url = format!("{}/torrents/{}/playlist", rqbit_base_url(), info_hash);
    let response = reqwest::Client::new()
        .get(&playlist_url)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|e| format!("Failed to request rqbit playlist: {e}"))?;

    if !response.status().is_success() {
        return Ok(Vec::new());
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read rqbit playlist: {e}"))?;

    Ok(parse_rqbit_playlist(&body, &rqbit_base_url()))
}

#[tauri::command]
async fn get_builtin_magnet_stats(info_hash: String) -> Result<BuiltinPlayerStats, String> {
    let stats_url = format!("{}/torrents/{}/stats/v1", rqbit_base_url(), info_hash);
    let response = reqwest::Client::new()
        .get(&stats_url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("Failed to request rqbit stats: {e}"))?;

    if !response.status().is_success() {
        return Ok(BuiltinPlayerStats {
            progress: None,
            download_speed: None,
            peers: None,
        });
    }

    let value = response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Failed to parse rqbit stats: {e}"))?;

    Ok(BuiltinPlayerStats {
        progress: find_numeric_field(&value, &["progress", "finished_percentage", "downloaded_percent"])
            .map(normalize_progress),
        download_speed: find_numeric_field(&value, &["download_speed", "download_speed_bytes", "downloaded_and_checked_bytes_per_second"])
            .map(|value| value.max(0.0) as u64),
        peers: find_numeric_field(&value, &["live", "peers", "num_peers", "connected_peers"])
            .map(|value| value.max(0.0) as u64),
    })
}

async fn ensure_daily_tracker_update(
    app_handle: &tauri::AppHandle,
    state: &tauri::State<'_, app_state::AppState>,
) -> Result<app_state::DownloadConfig, String> {
    let app_state = state.inner();
    let config = app_state::get_download_config(app_state);

    if !tracker_update_due(config.tracker_last_updated.as_deref()) {
        return Ok(config);
    }

    match fetch_tracker_servers(&config.tracker_sources).await {
        Ok(trackers) => {
            app_state::update_tracker_servers(app_state, trackers, chrono::Utc::now().to_rfc3339())
                .map_err(|e| e.to_string())?;
            app_state::save_app_state(app_handle, app_state).map_err(|e| e.to_string())?;
            Ok(app_state::get_download_config(app_state))
        }
        Err(error) => {
            eprintln!("Failed to update tracker servers, using cached list: {error}");
            Ok(config)
        }
    }
}

fn tracker_update_due(last_updated: Option<&str>) -> bool {
    let Some(last_updated) = last_updated else {
        return true;
    };

    let Ok(last_updated) = chrono::DateTime::parse_from_rfc3339(last_updated) else {
        return true;
    };

    chrono::Utc::now()
        .signed_duration_since(last_updated.with_timezone(&chrono::Utc))
        .num_hours()
        >= 24
}

async fn fetch_tracker_servers(sources: &[String]) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .user_agent("AI-Magnet-Assistant/1.2.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let mut trackers = app_state::default_tracker_servers();
    let mut seen: HashSet<String> = trackers.iter().cloned().collect();
    let mut errors = Vec::new();

    for source in sources {
        match client.get(source).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(response) => match response.text().await {
                    Ok(text) => {
                        for tracker in parse_tracker_list(&text) {
                            if seen.insert(tracker.clone()) {
                                trackers.push(tracker);
                            }
                        }
                    }
                    Err(error) => errors.push(format!("{source}: {error}")),
                },
                Err(error) => errors.push(format!("{source}: {error}")),
            },
            Err(error) => errors.push(format!("{source}: {error}")),
        }
    }

    if trackers.len() == app_state::default_tracker_servers().len() && !errors.is_empty() {
        return Err(format!("Failed to fetch tracker sources: {}", errors.join("; ")));
    }

    trackers.truncate(300);
    Ok(trackers)
}

fn parse_tracker_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.starts_with("udp://")
                || lower.starts_with("http://")
                || lower.starts_with("https://")
                || lower.starts_with("ws://")
                || lower.starts_with("wss://")
        })
        .map(ToString::to_string)
        .collect()
}

fn append_trackers_to_magnet(magnet_link: &str, trackers: &[String]) -> String {
    let mut result = magnet_link.to_string();
    let existing = magnet_link.to_ascii_lowercase();

    for tracker in trackers {
        if tracker.trim().is_empty() {
            continue;
        }

        let encoded_tracker = urlencoding::encode(tracker);
        if existing.contains(&format!("tr={}", encoded_tracker).to_ascii_lowercase()) {
            continue;
        }

        result.push_str("&tr=");
        result.push_str(&encoded_tracker);
    }

    result
}

fn rqbit_base_url() -> String {
    "http://127.0.0.1:3030".to_string()
}

fn extract_magnet_info_hash(magnet_link: &str) -> Result<String, String> {
    let re = Regex::new(r"(?i)(?:xt=urn:btih:|btih:)([a-z0-9]{32,40})")
        .map_err(|e| format!("Failed to compile info hash parser: {e}"))?;

    re.captures(magnet_link)
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str().to_ascii_uppercase())
        .ok_or_else(|| "Magnet link does not contain a BTIH info hash.".to_string())
}

async fn ensure_rqbit_server(app_handle: &tauri::AppHandle) -> Result<(), String> {
    if rqbit_server_ready().await {
        return Ok(());
    }

    let executable = find_rqbit_executable(app_handle).ok_or_else(|| {
        "找不到本地 rqbit 播放引擎。请将 rqbit.exe 放到程序同目录，或设置 RQBIT_PATH 指向 rqbit 可执行文件。".to_string()
    })?;

    let download_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data directory: {e}"))?
        .join("rqbit-downloads");

    std::fs::create_dir_all(&download_dir)
        .map_err(|e| format!("Failed to create rqbit download directory: {e}"))?;

    Command::new(executable)
        .args(["server", "start"])
        .arg(download_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start rqbit player engine: {e}"))?;

    for _ in 0..30 {
        if rqbit_server_ready().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    Err("rqbit 播放引擎已启动但未在 127.0.0.1:3030 响应。".to_string())
}

async fn rqbit_server_ready() -> bool {
    reqwest::Client::new()
        .get(rqbit_base_url())
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

fn find_rqbit_executable(app_handle: &tauri::AppHandle) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("RQBIT_PATH") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    let exe_names = if cfg!(target_os = "windows") {
        vec!["rqbit.exe"]
    } else {
        vec!["rqbit"]
    };

    let mut candidate_dirs = Vec::new();
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            candidate_dirs.push(parent.to_path_buf());
        }
    }
    if let Ok(resource_dir) = app_handle.path().resource_dir() {
        candidate_dirs.push(resource_dir);
    }

    for dir in candidate_dirs {
        for exe_name in &exe_names {
            let candidate = dir.join(exe_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    if let Some(extracted) = extract_bundled_rqbit(app_handle) {
        return Some(extracted);
    }

    Some(PathBuf::from(exe_names[0]))
}

#[cfg(target_os = "windows")]
fn extract_bundled_rqbit(app_handle: &tauri::AppHandle) -> Option<PathBuf> {
    const RQBIT_BYTES: &[u8] = include_bytes!("../binaries/rqbit-x86_64-pc-windows-msvc.exe");

    let engine_dir = app_handle
        .path()
        .app_data_dir()
        .ok()?
        .join("player-engine");
    let engine_path = engine_dir.join("rqbit.exe");

    std::fs::create_dir_all(&engine_dir).ok()?;

    let should_write = std::fs::metadata(&engine_path)
        .map(|metadata| metadata.len() != RQBIT_BYTES.len() as u64)
        .unwrap_or(true);

    if should_write {
        std::fs::write(&engine_path, RQBIT_BYTES).ok()?;
    }

    Some(engine_path)
}

#[cfg(not(target_os = "windows"))]
fn extract_bundled_rqbit(_app_handle: &tauri::AppHandle) -> Option<PathBuf> {
    None
}

async fn add_magnet_to_rqbit(magnet_link: &str) -> Result<(), String> {
    let response = reqwest::Client::new()
        .post(format!("{}/torrents", rqbit_base_url()))
        .query(&[("overwrite", "false")])
        .body(magnet_link.to_string())
        .timeout(Duration::from_secs(12))
        .send()
        .await
        .map_err(|e| format!("Failed to add magnet to rqbit: {e}"))?;

    if response.status().is_success() || response.status().as_u16() == 409 {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!("rqbit rejected the magnet link: HTTP {status} {body}"))
}

fn parse_rqbit_playlist(body: &str, base_url: &str) -> Vec<BuiltinPlayerFile> {
    let mut files = Vec::new();
    let mut current_name: Option<String> = None;

    for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with("#EXTINF") {
            current_name = line
                .split_once(',')
                .map(|(_, name)| name.trim().to_string())
                .filter(|name| !name.is_empty());
            continue;
        }

        if line.starts_with('#') {
            continue;
        }

        let url = if line.starts_with("http://") || line.starts_with("https://") {
            line.to_string()
        } else if line.starts_with('/') {
            format!("{base_url}{line}")
        } else {
            format!("{base_url}/{line}")
        };

        let fallback_name = url
            .rsplit('/')
            .next()
            .and_then(|part| part.split('?').next())
            .filter(|part| !part.is_empty())
            .unwrap_or("media")
            .to_string();
        let name = current_name.take().unwrap_or(fallback_name);

        files.push(BuiltinPlayerFile {
            media_type: infer_media_type(&name),
            name,
            url,
            length: None,
        });
    }

    files
}

fn infer_media_type(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".mp3")
        || lower.ends_with(".ogg")
        || lower.ends_with(".wav")
        || lower.ends_with(".flac")
        || lower.ends_with(".aac")
        || lower.ends_with(".m4a")
    {
        "audio".to_string()
    } else {
        "video".to_string()
    }
}

fn normalize_progress(value: f64) -> f64 {
    if value > 1.0 {
        (value / 100.0).clamp(0.0, 1.0)
    } else {
        value.clamp(0.0, 1.0)
    }
}

fn find_numeric_field(value: &serde_json::Value, names: &[&str]) -> Option<f64> {
    match value {
        serde_json::Value::Object(map) => {
            for name in names {
                if let Some(number) = map.get(*name).and_then(serde_json::Value::as_f64) {
                    return Some(number);
                }
            }
            map.values().find_map(|child| find_numeric_field(child, names))
        }
        serde_json::Value::Array(items) => items.iter().find_map(|child| find_numeric_field(child, names)),
        _ => None,
    }
}

fn open_magnet_with_default_app(magnet_link: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", magnet_link])
            .spawn()
            .map_err(|e| format!("Failed to open magnet link with the system default app: {e}"))?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(magnet_link)
            .spawn()
            .map_err(|e| format!("Failed to open magnet link with the system default app: {e}"))?;
        return Ok(());
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(magnet_link)
            .spawn()
            .map_err(|e| format!("Failed to open magnet link with the system default app: {e}"))?;
        Ok(())
    }
}

fn open_magnet_with_app(magnet_link: &str, app_path: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let path = std::path::Path::new(app_path);
        let mut command = if app_path.ends_with(".app") || path.is_dir() {
            let mut command = std::process::Command::new("open");
            command.arg("-a").arg(app_path).arg(magnet_link);
            command
        } else {
            let mut command = std::process::Command::new(app_path);
            command.arg(magnet_link);
            command
        };

        command
            .spawn()
            .map_err(|e| format!("Failed to open magnet link with the selected app: {e}"))?;
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    {
        std::process::Command::new(app_path)
            .arg(magnet_link)
            .spawn()
            .map_err(|e| format!("Failed to open magnet link with the selected app: {e}"))?;
        Ok(())
    }
}

fn open_file_with_app(file_path: &std::path::Path, app_path: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let path = std::path::Path::new(app_path);
        let mut command = if app_path.ends_with(".app") || path.is_dir() {
            let mut command = std::process::Command::new("open");
            command.arg("-a").arg(app_path).arg(file_path);
            command
        } else {
            let mut command = std::process::Command::new(app_path);
            command.arg(file_path);
            command
        };

        command
            .spawn()
            .map_err(|e| format!("Failed to open temporary download page: {e}"))?;
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    {
        std::process::Command::new(app_path)
            .arg(file_path)
            .spawn()
            .map_err(|e| format!("Failed to open temporary download page: {e}"))?;
        Ok(())
    }
}

async fn create_and_open_magnet_html(magnet_link: &str, browser_path: &str, config: &app_state::DownloadConfig) -> Result<(), String> {
    use std::fs;

    // 创建临时目录
    let temp_dir = std::env::temp_dir();
    let html_file = temp_dir.join("magnet_download.html");

    // 创建HTML内容，包含磁力链接
    let auto_close_script = if config.auto_close_page {
        r#"
        // 自动关闭页面
        setTimeout(function() {
            window.close();
        }, 10000);
        "#.to_string()
    } else {
        String::new()
    };

    let close_info = if config.auto_close_page {
        "This page will close automatically in 10 seconds.".to_string()
    } else {
        "You can close this page manually.".to_string()
    };

    let html_content = format!(r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>115 Offline Download</title>
    <style>
        body {{
            font-family: Arial, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
        }}
        .container {{
            text-align: center;
            background: white;
            padding: 40px;
            border-radius: 15px;
            box-shadow: 0 10px 30px rgba(0,0,0,0.2);
            max-width: 500px;
        }}
        .success-icon {{
            font-size: 48px;
            color: #28a745;
            margin-bottom: 20px;
        }}
        .magnet-link {{
            display: inline-block;
            padding: 12px 24px;
            background: #3b82f6;
            color: white;
            text-decoration: none;
            border-radius: 8px;
            font-size: 16px;
            font-weight: 600;
            margin: 20px 0;
            transition: all 0.2s;
            border: none;
            cursor: pointer;
        }}
        .magnet-link:hover {{
            background: #2563eb;
            transform: translateY(-1px);
            box-shadow: 0 4px 12px rgba(59, 130, 246, 0.3);
        }}
        .status {{
            color: #28a745;
            font-weight: bold;
            font-size: 18px;
            margin: 20px 0;
        }}
        .info {{
            color: #666;
            margin-top: 20px;
            font-size: 14px;
        }}
        .countdown {{
            color: #007bff;
            font-weight: bold;
        }}
    </style>
    <script>
        let countdown = 10;

        // 自动点击磁力链接
        window.onload = function() {{
            setTimeout(function() {{
                document.getElementById('magnetLink').click();
                document.getElementById('status').innerHTML = '✅ Download started successfully!';

                // 开始倒计时
                if (countdown > 0) {{
                    updateCountdown();
                }}
            }}, 1000);
        }};

        function updateCountdown() {{
            if (countdown > 0) {{
                document.getElementById('countdown').innerHTML = countdown;
                countdown--;
                setTimeout(updateCountdown, 1000);
            }}
        }}

        {}
    </script>
</head>
<body>
    <div class="container">
        <div class="success-icon">📥</div>
        <h2>115 Offline Download</h2>
        <div id="status" class="status">Starting download...</div>
        <p>If download doesn't start automatically, click below:</p>
        <a href="{}" id="magnetLink" class="magnet-link">Manual Download</a>
        <p class="info">{}</p>
        <p class="info countdown" id="countdown-info">
            <span id="countdown">{}</span>
        </p>
    </div>
</body>
</html>
"#,
    auto_close_script,
    magnet_link,
    close_info,
    if config.auto_close_page { "10" } else { "" }
);

    // 写入HTML文件
    fs::write(&html_file, html_content)
        .map_err(|e| format!("Failed to create temporary HTML file: {e}"))?;

    // 使用115浏览器打开HTML文件
    open_file_with_app(&html_file, browser_path)?;

    // 等待一下让浏览器启动
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 异步删除临时文件（延迟删除，确保浏览器已经读取）
    let html_file_clone = html_file.clone();
    tokio::spawn(async move {
        // 等待足够长的时间确保浏览器已经加载了文件
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        let _ = std::fs::remove_file(html_file_clone);
    });

    Ok(())
}

#[tauri::command]
async fn browse_for_file() -> Result<Option<String>, String> {
    use std::process::Command;

    // 使用Windows的文件对话框
    #[cfg(target_os = "windows")]
    {
        let output = Command::new("powershell")
            .args([
                "-Command",
                r#"
                Add-Type -AssemblyName System.Windows.Forms
                $dialog = New-Object System.Windows.Forms.OpenFileDialog
                $dialog.Filter = "Executable files (*.exe)|*.exe|All files (*.*)|*.*"
                $dialog.Title = "Select Application"
                if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
                    $dialog.FileName
                }
                "#
            ])
            .output()
            .map_err(|e| format!("Failed to open file dialog: {e}"))?;

        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if path.is_empty() {
                Ok(None)
            } else {
                Ok(Some(path))
            }
        } else {
            Err("File dialog was cancelled or failed".to_string())
        }
    }

    #[cfg(target_os = "macos")]
    {
        let output = Command::new("osascript")
            .args([
                "-e",
                r#"POSIX path of (choose application with prompt "Select Application")"#,
            ])
            .output()
            .map_err(|e| format!("Failed to open application picker: {e}"))?;

        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if path.is_empty() {
                Ok(None)
            } else {
                Ok(Some(path))
            }
        } else {
            Err("Application picker was cancelled or failed".to_string())
        }
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        Err("File browser is only supported on Windows and macOS".to_string())
    }
}

// ============ 语言状态管理命令 ============

#[tauri::command]
async fn get_app_locale(state: tauri::State<'_, app_state::AppState>) -> Result<String, String> {
    Ok(app_state::get_current_locale(&state))
}

#[tauri::command]
async fn set_app_locale_with_persistence(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, app_state::AppState>,
    locale: String,
) -> Result<(), String> {
    // 设置后端国际化模块的语言
    i18n::get_i18n_manager()
        .set_locale(&locale)
        .map_err(|e| e.to_string())?;
    
    // 保存到应用状态
    app_state::set_current_locale(&state, locale.clone())
        .map_err(|e| e.to_string())?;
    
    // 持久化到文件
    app_state::save_app_state(&app_handle, &state)
        .map_err(|e| e.to_string())?;
    
    println!("📝 语言设置已更新并持久化: {locale}");
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // 初始化应用状态
            let app_state = app_state::init_app_state(app.handle())
                .expect("Failed to initialize app state");
            app.manage(app_state);

            let tracker_app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = tracker_app_handle.state::<app_state::AppState>();
                let _ = ensure_daily_tracker_update(&tracker_app_handle, &state).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            search_multi_page,
            search_clmclm_first,
            search_other_engines,
            test_connection,
            test_extraction_connection,
            test_analysis_connection,
            analyze_resource,
            batch_analyze_resources,
            // 收藏夹命令
            add_to_favorites,
            get_all_favorites,
            remove_from_favorites,
            search_favorites,
            // 搜索引擎命令
            add_search_engine,
            update_search_engine,
            get_all_engines,
            update_engine_status,
            delete_engine,
            // 优先关键词命令
            add_priority_keyword,
            get_all_priority_keywords,
            delete_priority_keyword,
            // LLM 配置命令
            get_llm_config,
            update_llm_config,
            // 搜索设置命令
            get_search_settings,
            update_search_settings,
            // 下载配置命令
            get_download_config,
            update_download_config,
            refresh_tracker_servers,
            open_magnet_link,
            play_magnet_link,
            start_builtin_magnet_player,
            get_builtin_magnet_playlist,
            get_builtin_magnet_stats,
            browse_for_file,
            // 国际化命令
            i18n::get_system_locale,
            i18n::set_app_locale,
            i18n::get_current_locale,
            i18n::get_supported_locales,
            i18n::get_localized_message,
            // 语言状态管理命令
            get_app_locale,
            set_app_locale_with_persistence
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
