<template>
  <div class="home-page">
    <div class="page-header">
      <h1>{{ $t('pages.home.title') }}</h1>
      <p>{{ $t('pages.home.subtitle') }}</p>
    </div>

    <div class="search-section">
      <div class="search-row">
        <input
          v-model="keyword"
          :placeholder="$t('pages.home.search.placeholder')"
          @keyup.enter="search"
          class="search-input"
        />
        <button @click="search" class="search-btn">
          {{ $t('pages.home.search.button.search') }}
        </button>
      </div>

      <div class="filter-options">
        <div class="pages-selector">
          <label for="maxPages">{{ $t('pages.home.search.filters.pages') }}:</label>
          <select id="maxPages" v-model="maxPages">
            <option :value="1">{{ $t('pages.home.search.filters.pageOptions.one') }}</option>
            <option :value="3">{{ $t('pages.home.search.filters.pageOptions.three') }}</option>
            <option :value="5">{{ $t('pages.home.search.filters.pageOptions.five') }}</option>
            <option :value="10">{{ $t('pages.home.search.filters.pageOptions.ten') }}</option>
          </select>
        </div>

        <label class="checkbox-label">
          <input type="checkbox" v-model="useSmartFilter" />
          <span>{{ $t('pages.home.search.filters.aiFilter') }}</span>
        </label>

        <label class="checkbox-label">
          <input type="checkbox" v-model="titleMustContainKeyword" />
          <span>{{ $t('pages.home.search.filters.titleMustContainKeyword') }}</span>
        </label>
      </div>

      <div v-if="searchStatus" class="status">
        {{ searchStatus }}
      </div>
    </div>

    <div v-if="results.length > 0" class="results-section">
      <div class="results-header">
        <h2>{{ $t('pages.home.results.title') }} {{ $t('pages.home.results.count', { count: results.length }) }}</h2>
        <div class="sort-controls">
          <label for="sortBy">{{ $t('pages.home.results.sortBy') }}:</label>
          <select id="sortBy" v-model="sortBy" @change="onSortChange" class="sort-selector">
            <option value="score">{{ $t('pages.home.results.sortOptions.score') }}</option>
            <option value="size">{{ $t('pages.home.results.sortOptions.size') }}</option>
          </select>
        </div>
      </div>
      <div class="results-grid">
        <div v-for="result in results" :key="result.magnet_link || result.source_url || result.title" class="result-item-wrapper">
          <ResultCard
            :title="result.title"
            :original-title="result.originalTitle"
            :magnet-link="result.magnet_link"
            :file-size="result.file_size"
            :upload-date="result.upload_date"
            :analysis="result.analysis"
            :is-priority="result.isPriority"
            :file-list="result.file_list"
            :source-engine="result.source_engine"
            :source-url="result.source_url"
            :preview-image-url="result.preview_image_url"
            @add-to-favorites="addToFavorites"
            @show-notification="handleNotification"
          />
          <div v-if="result.analysis && result.analysis.error" class="error-details">
            <div class="error-header" @click="toggleErrorExpanded(result)">
              <strong>Error:</strong>
              <span class="error-preview">{{ getErrorPreview(result.analysis.error) }}</span>
              <span class="error-toggle">{{ result.errorExpanded ? '▼' : '▶' }}</span>
            </div>
            <div v-if="result.errorExpanded" class="error-full">
              {{ result.analysis.error }}
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, inject, computed } from 'vue';
import { invoke } from "@tauri-apps/api/core";
import ResultCard from './ResultCard.vue';
import { useI18n } from '../composables/useI18n';
import { logger } from '../utils/logger';

const { t } = useI18n();

const showNotification = inject('showNotification') as any; // Correct position
const favoritesTimestamp = inject('favoritesTimestamp') as any;

// Inject global search state
const searchState = inject('searchState') as any;

// Use global state, create local state if not available
const keyword = searchState ? computed({
  get: () => searchState.value.keyword,
  set: (val) => searchState.value.keyword = val
}) : ref("");

const results = searchState ? computed({
  get: () => searchState.value.results,
  set: (val) => searchState.value.results = val
}) : ref([]);

const isSearching = searchState ? computed({
  get: () => searchState.value.isSearching,
  set: (val) => searchState.value.isSearching = val
}) : ref(false);

const searchStatus = searchState ? computed({
  get: () => searchState.value.searchStatus,
  set: (val) => searchState.value.searchStatus = val
}) : ref("");

const useSmartFilter = searchState ? computed({
  get: () => searchState.value.useSmartFilter,
  set: (val) => searchState.value.useSmartFilter = val
}) : ref(true);

const maxPages = searchState ? computed({
  get: () => searchState.value.maxPages,
  set: (val) => searchState.value.maxPages = val
}) : ref(1);

const sortBy = searchState ? computed({
  get: () => searchState.value.sortBy,
  set: (val) => {
    searchState.value.sortBy = val
    try { localStorage.setItem('search-sort-by', String(val)) } catch {}
  }
}) : ref('score');

// 搜索取消机制
let currentSearchId = 0;

const titleMustContainKeyword = searchState ? computed({
  get: () => searchState.value.titleMustContainKeyword,
  set: (val) => searchState.value.titleMustContainKeyword = val
}) : ref(true);




// Sort function
async function sortResults(resultsArray: any[]) {
  // First get priority keywords
  let priorityKeywords: any[] = [];
  try {
    priorityKeywords = await invoke("get_all_priority_keywords");
  } catch (error) {
    logger.error("Failed to load priority keywords:", error);
  }

  // Add priority flag to each result
  const sortedResults = [...resultsArray];

  sortedResults.forEach((result: any) => {
    result.isPriority = priorityKeywords.some((pk: any) => {
      const keyword = pk.keyword.toLowerCase();
      // Check title
      if (String(result.title || '').toLowerCase().includes(keyword)) {
        return true;
      }
      // Check file list
      if (result.file_list && Array.isArray(result.file_list)) {
        return result.file_list.some((fileName: string) => fileName.toLowerCase().includes(keyword));
      }
      return false;
    });
  });

  // Sort: priority keyword results first, then by selected sort method
  sortedResults.sort((a: any, b: any) => {
    // First sort by priority
    if (a.isPriority && !b.isPriority) return -1;
    if (!a.isPriority && b.isPriority) return 1;

    // If priority is the same, sort by selected method
    if (sortBy.value === 'score') {
      const scoreA = a.analysis?.purity_score || 0;
      const scoreB = b.analysis?.purity_score || 0;
      if (scoreB !== scoreA) return scoreB - scoreA;
    } else if (sortBy.value === 'size') {
      const sizeA = parseSizeToBytes(a.file_size || '0');
      const sizeB = parseSizeToBytes(b.file_size || '0');
      if (sizeB !== sizeA) return sizeB - sizeA;
    }

    const dateA = Date.parse(a.upload_date || '') || 0;
    const dateB = Date.parse(b.upload_date || '') || 0;
    if (dateB !== dateA) return dateB - dateA;

    return String(a.title || '').localeCompare(String(b.title || ''));
  });

  results.value = sortedResults;
}

function parseSizeToBytes(sizeStr: string): number {
  if (!sizeStr) return 0;
  const normalized = sizeStr.replace(/,/g, '').trim();
  const match = normalized.match(/^([\d.]+)\s*([KMGT]i?B|[KMGT]?B)$/i);
  if (!match) return 0;
  
  const value = parseFloat(match[1]);
  const unit = match[2].toUpperCase().replace('IB', 'B');
  
  const multipliers: { [key: string]: number } = {
    'B': 1,
    'KB': 1024,
    'MB': 1024 * 1024,
    'GB': 1024 * 1024 * 1024,
    'TB': 1024 * 1024 * 1024 * 1024
  };
  
  return value * (multipliers[unit] || 1);
}

function filterResultsByKeyword(resultsArray: any[]) {
  if (!titleMustContainKeyword.value) {
    return resultsArray;
  }

  const normalizedKeyword = keyword.value.trim().toLowerCase();
  if (!normalizedKeyword) {
    return resultsArray;
  }

  const keywordParts = normalizedKeyword
    .split(/\s+/)
    .map((part: string) => part.trim())
    .filter(Boolean);

  const filteredResults = resultsArray.filter((result: any) => {
    const title = String(result.title || '').toLowerCase();
    const originalTitle = String(result.originalTitle || '').toLowerCase();
    const fileList = Array.isArray(result.file_list)
      ? result.file_list.join(' ').toLowerCase()
      : '';
    const displayName = String(result.magnet_link || '').toLowerCase();
    const searchableText = `${title} ${originalTitle} ${fileList} ${displayName}`;

    return keywordParts.every((part: string) => searchableText.includes(part));
  });

  if (filteredResults.length === 0 && resultsArray.length > 0) {
    logger.debug(
      `Title keyword filter would hide all ${resultsArray.length} results for "${normalizedKeyword}", showing unfiltered results instead`
    );
    return resultsArray;
  }

  return filteredResults;
}

function formatSearchError(engine: string, error: unknown) {
  const message = String(error || 'Unknown error').replace(/^Error:\s*/, '');
  return `${engine}: ${message}`;
}

async function onSortChange() {
  await sortResults(results.value);
}

async function search() {
  // 初始化 sortBy（首次进入页面或无状态时从本地存储恢复）
  if (!searchState && typeof window !== 'undefined') {
    const saved = localStorage.getItem('search-sort-by')
    if (saved === 'score' || saved === 'size') {
      ;(sortBy as any).value = saved
    }
  }
  if (!keyword.value.trim()) {
    alert(t('pages.home.messages.emptyKeyword'));
    return;
  }

  // 生成新的搜索ID，取消之前的搜索
  const searchId = ++currentSearchId;

  // 检查搜索是否被取消的函数
  const isSearchCancelled = () => currentSearchId !== searchId;

  isSearching.value = true;
  results.value = [];
  const searchErrors: string[] = [];

  try {
    // Load LLM config and enabled engines to determine if AI will be used
    const llmConfig = await invoke("get_llm_config") as any;
    const engines = await invoke("get_all_engines") as any[];
    const enabledEngines = engines.filter((e: any) => e.is_enabled);
    const hasOtherEnabledEngines = enabledEngines.some((e: any) => e.name !== "clmclm.com");

    // Check if AI will be used for HTML extraction
    const hasCustomEngines = enabledEngines.some((e: any) => e.name !== "clmclm.com");
    const hasExtractionConfig = llmConfig?.extraction_config?.api_key;
    const willUseAI = hasCustomEngines && hasExtractionConfig;

    // Display model information only if AI will be used
    let modelInfo = "";
    if (willUseAI) {
      const extractionModel = llmConfig.extraction_config?.model || "Not configured";
      modelInfo = ` (using ${extractionModel} for HTML extraction)`;
    }

    searchStatus.value = t('pages.home.search.status.searchingWithModel', { modelInfo });

    const clmclmPromise = invoke("search_clmclm_first", {
      keyword: keyword.value,
      maxPages: maxPages.value
    });

    const otherEnginesPromise = invoke("search_other_engines", {
      keyword: keyword.value,
      maxPages: maxPages.value
    });

    const [clmclmSettled, otherSettled] = await Promise.allSettled([clmclmPromise, otherEnginesPromise]);

    if (isSearchCancelled()) {
      logger.debug('Search cancelled after search results');
      return;
    }

    const mergedResults: any[] = [];

    if (clmclmSettled.status === 'fulfilled') {
      mergedResults.push(...filterResultsByKeyword((clmclmSettled.value as any[]) || []));
    } else {
      logger.debug('clmclm search failed:', clmclmSettled.reason);
      searchErrors.push(formatSearchError('clmclm.com', clmclmSettled.reason));
    }

    if (otherSettled.status === 'fulfilled') {
      mergedResults.push(...filterResultsByKeyword((otherSettled.value as any[]) || []));
    } else if (hasOtherEnabledEngines) {
      logger.debug('Other engines search failed:', otherSettled.reason);
      searchErrors.push(formatSearchError('Other engines', otherSettled.reason));
    }

    results.value = dedupeResults(mergedResults);
    await sortResults(results.value);

    searchStatus.value = t('pages.home.search.status.completeWithCount', {
      count: results.value.length,
      modelInfo
    });

    if (useSmartFilter.value && results.value.length > 0) {
      if (isSearchCancelled()) {
        logger.debug('Search cancelled before analysis');
        return;
      }
      searchStatus.value = t('pages.home.search.status.analyzingWithModel', { modelInfo });
      await analyzeResults();
      await sortResults(results.value);
    }

    // 最终检查搜索是否被取消
    if (isSearchCancelled()) {
      logger.debug('Search cancelled before final status');
      return;
    }

    // 最终状态 - 如果没有启用智能过滤或没有进行分析，显示基本搜索完成状态
    if (!useSmartFilter.value || results.value.length === 0) {
      if (results.value.length === 0 && searchErrors.length > 0) {
        searchStatus.value = t('pages.home.search.status.failed', { reason: searchErrors.join(' | ') });
        return;
      }

      searchStatus.value = t('pages.home.search.status.completeWithCount', { 
        count: results.value.length,
        modelInfo 
      });
    }
    // 如果启用了智能过滤并且有结果，analyzeResults() 已经设置了包含分析信息的最终状态，不要覆盖它

  } catch (error) {
    logger.error("Search failed:", error);
    searchStatus.value = t('pages.home.search.status.failed', { reason: String(error) });
  } finally {
    // 只有当前搜索才能重置搜索状态
    if (!isSearchCancelled()) {
      isSearching.value = false;
    }
  }
}

function dedupeResults(resultsArray: any[]) {
  const seen = new Set<string>();
  return resultsArray.filter((result: any) => {
    const key = result.magnet_link || result.source_url || result.title;
    if (!key || seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

async function analyzeResults() {
  try {
    // Load LLM config
    const llmConfig = await invoke("get_llm_config") as any;
    if (!llmConfig || !llmConfig.analysis_config?.api_key) {
      searchStatus.value = t('pages.home.search.status.requiresApiKey');
      return;
    }

    const startTime = Date.now();
    const analysisModel = llmConfig.analysis_config?.model || "Unknown";
    const configuredBatchSize = Number(llmConfig.analysis_config?.batch_size || 5);
    const provider = String(llmConfig.analysis_config?.provider || '').toLowerCase();
    const batchSize = Math.max(1, Math.min(configuredBatchSize || 5, provider === 'deepseek' ? 3 : 10));
    const maxConcurrentBatches = provider === 'deepseek' ? 1 : 2;

    // 只分析尚未分析的结果
    const unanalyzedResults = results.value.filter((result: any) => !result.analysis);
    const alreadyAnalyzedCount = results.value.length - unanalyzedResults.length;

    if (unanalyzedResults.length === 0) {
      logger.debug('All results already analyzed, skipping analysis');
      return;
    }

    logger.debug(`Frontend AI analysis: ${unanalyzedResults.length} unanalyzed results (${results.value.length} total, ${alreadyAnalyzedCount} already analyzed), batch_size=${batchSize}, model=${analysisModel}`);

    let completedCount = alreadyAnalyzedCount;
    let hasErrors = false;
    let errorMessages: string[] = [];

    const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
    const applyAnalysisResult = (result: any, analysis: any, batchIndex: number, itemIndex: number) => {
      if (analysis && !analysis.error) {
        result.analysis = {
          title: analysis.title,
          purity_score: analysis.purity_score,
          tags: analysis.tags,
        };

        if (!result.originalTitle) {
          result.originalTitle = result.title;
        }
        result.title = analysis.title;
      } else {
        const errorMsg = analysis?.error || 'No analysis result returned';
        result.analysis = {
          error: errorMsg,
          title: result.title,
          purity_score: 0,
          tags: ['Analysis Failed']
        };
        hasErrors = true;
        errorMessages.push(`Batch ${batchIndex + 1} item ${itemIndex + 1} failed: ${errorMsg}`);
        logger.debug(`Set error for result "${result.title}": ${errorMsg}`);
      }

      completedCount++;
      const errorCount = errorMessages.length;
      const errorSuffix = errorCount > 0 ? `, ${errorCount} errors` : '';
      searchStatus.value = t('pages.home.search.status.analyzingParallel', {
        batches: Math.ceil(unanalyzedResults.length / batchSize),
        completed: completedCount,
        total: results.value.length,
        errorSuffix,
        model: analysisModel
      });
    };

    // 使用受控并发。DeepSeek 对大量并发请求更敏感，过高并发会导致整批失败。
    try {
      const totalBatches = Math.ceil(unanalyzedResults.length / batchSize);
      logger.debug(`Starting ${totalBatches} batches with batch_size=${batchSize}, concurrency=${maxConcurrentBatches}, provider=${provider || 'unknown'}`);

      const batches: Array<{ index: number; results: any[] }> = [];
      for (let batchIndex = 0; batchIndex < totalBatches; batchIndex++) {
        const startIdx = batchIndex * batchSize;
        const endIdx = Math.min(startIdx + batchSize, unanalyzedResults.length);
        batches.push({
          index: batchIndex,
          results: unanalyzedResults.slice(startIdx, endIdx),
        });
      }

      const processBatch = async (batch: { index: number; results: any[] }) => {
        const batchIndex = batch.index;
        const batchResults = batch.results;
          try {
            logger.debug(`Starting batch ${batchIndex + 1}/${totalBatches} with ${batchResults.length} items`);

            const analysisResults = await invoke('batch_analyze_resources', {
              results: batchResults
            });

            // 将分析结果应用到原始结果中
            if (Array.isArray(analysisResults)) {
              for (let i = 0; i < batchResults.length; i++) {
                const result = batchResults[i];
                const analysis = analysisResults[i];
                applyAnalysisResult(result, analysis, batchIndex, i);
              }

              logger.debug(`Batch ${batchIndex + 1}/${totalBatches} completed: ${analysisResults.length} results processed`);
              return { success: true, batchIndex: batchIndex + 1, count: analysisResults.length };
            } else {
              // 批量分析返回了非数组结果，视为失败
              throw new Error('Batch analysis returned invalid result format');
            }
          } catch (batchError) {
            logger.error(`Batch ${batchIndex + 1} failed, falling back to sequential individual analysis:`, batchError);
            hasErrors = true;
            errorMessages.push(`Batch ${batchIndex + 1} failed: ${batchError}`);

            // 对这个批次回退到单个分析。这里必须顺序执行，避免失败后继续放大限流。
            for (let i = 0; i < batchResults.length; i++) {
              const result = batchResults[i];
              try {
                const analysisConfig = {
                  provider: llmConfig.analysis_config.provider,
                  api_key: llmConfig.analysis_config.api_key,
                  api_base: llmConfig.analysis_config.api_base,
                  model: llmConfig.analysis_config.model,
                  batch_size: llmConfig.analysis_config.batch_size,
                };

                if (provider === 'deepseek') {
                  await sleep(700);
                }

                const rawAnalysis = await invoke('analyze_resource', {
                  result: result,
                  llmConfig: analysisConfig,
                });

                let analysis;
                try {
                  if (typeof rawAnalysis === 'string') {
                    analysis = JSON.parse(rawAnalysis);
                  } else {
                    analysis = rawAnalysis;
                  }
                } catch (e) {
                  logger.error('Failed to parse analysis from backend:', e);
                  analysis = { error: `Failed to parse analysis: ${e}` };
                }

                applyAnalysisResult(result, analysis, batchIndex, i);
                const errorCount = errorMessages.length;
                const errorSuffix = errorCount > 0 ? `, ${errorCount} errors` : '';
                searchStatus.value = t('pages.home.search.status.individualFallback', { 
                  completed: completedCount,
                  total: results.value.length,
                  errorSuffix,
                  model: analysisModel 
                });

              } catch (e) {
                logger.error(`Failed to analyze result: ${result.title}`, e);
                const errorMsg = `Analysis Failed: ${e}`;
                result.analysis = {
                  error: errorMsg,
                  title: result.title, // 保持原标题
                  purity_score: 0,
                  tags: ['Analysis Failed']
                };
                hasErrors = true;
                errorMessages.push(`Individual analysis failed for "${result.title}": ${e}`);
                completedCount++;

                logger.debug(`Set individual error for result "${result.title}": ${errorMsg}`);

                // 实时更新状态显示错误
                searchStatus.value = t('pages.home.search.status.individualFallback', { 
                  completed: completedCount,
                  total: results.value.length,
                  errorSuffix: `, ${errorMessages.length} errors`,
                  model: analysisModel 
                });
              }
            }
            return { success: false, batchIndex: batchIndex + 1, error: batchError };
          }
      };

      searchStatus.value = t('pages.home.search.status.batchAnalysis', { 
        batches: totalBatches,
        completed: alreadyAnalyzedCount,
        total: results.value.length,
        model: analysisModel 
      });

      const batchResults: any[] = [];
      let nextBatchIndex = 0;
      const workers = Array.from({ length: Math.min(maxConcurrentBatches, batches.length) }, async () => {
        while (nextBatchIndex < batches.length) {
          const batch = batches[nextBatchIndex++];
          batchResults.push(await processBatch(batch));
          if (provider === 'deepseek') {
            await sleep(600);
          }
        }
      });
      await Promise.all(workers);

      const successfulBatches = batchResults.filter((r: any) => r && r.success).length;
      const failedBatches = batchResults.filter((r: any) => r && !r.success).length;

      logger.debug(`All ${totalBatches} parallel batches completed: ${successfulBatches} successful, ${failedBatches} failed, ${completedCount} results processed`);
    } catch (e) {
      logger.error('Complete parallel analysis failed:', e);
      hasErrors = true;
      errorMessages.push(`Complete analysis failed: ${e}`);
      searchStatus.value = t('pages.home.search.status.failed', { reason: String(e) });
    }

    // 显示最终状态
    const endTime = Date.now();
    const duration = ((endTime - startTime) / 1000).toFixed(1);

    if (hasErrors && errorMessages.length > 0) {
      searchStatus.value = t('pages.home.search.status.completeWithErrors', { 
        duration,
        count: completedCount,
        errors: errorMessages.length,
        model: analysisModel 
      });
    } else {
      searchStatus.value = t('pages.home.search.status.completeWithAnalysis', { 
        duration,
        count: completedCount,
        model: analysisModel 
      });
    }
  } catch (error) {
    logger.error('AI analysis failed:', error);
    searchStatus.value = t('pages.home.search.status.failed', { reason: String(error) });
  }
}

async function addToFavorites(result: any) {
  try {
    await invoke("add_to_favorites", {
      title: result.title,
      magnetLink: result.magnet_link,
      fileSize: result.file_size,
      fileList: result.file_list || [],
    });
    showNotification(t('pages.home.messages.addedToFavorites'), "success");
    favoritesTimestamp.value = Date.now(); // 触发刷新
  } catch (error) {
    logger.error("Failed to add to favorites:", error);
    showNotification(t('pages.home.messages.failedToAddFavorites', { error: String(error) }), "error");
  }
}

function toggleErrorExpanded(result: any) {
  result.errorExpanded = !result.errorExpanded;
}

function handleNotification(message: string, type?: 'success' | 'error') {
  showNotification(message, type);
}



function getErrorPreview(errorMessage: string): string {
  if (!errorMessage) return '';

  // 提取关键错误信息
  if (errorMessage.includes('rate limit') || errorMessage.includes('quota')) {
    return t('pages.home.errors.apiRateLimit');
  } else if (errorMessage.includes('timeout')) {
    return t('pages.home.errors.requestTimeout');
  } else if (errorMessage.includes('network') || errorMessage.includes('connection')) {
    return t('pages.home.errors.networkError');
  } else if (errorMessage.includes('parse') || errorMessage.includes('JSON')) {
    return t('pages.home.errors.responseParsingError');
  } else {
    // 截取前50个字符作为预览
    return errorMessage.length > 50 ? errorMessage.substring(0, 50) + '...' : errorMessage;
  }
}


</script>

<style scoped>
.home-page {
  padding: 24px;
  width: 100%;
  box-sizing: border-box;
  overflow-x: hidden;
  min-width: 0;
}

/* 响应式padding调整 */
@media (max-width: 1200px) {
  .home-page {
    padding: 16px;
  }
}

@media (max-width: 768px) {
  .home-page {
    padding: 12px;
  }
}

.page-header {
  margin-bottom: 32px;
}

.page-header h1 {
  margin: 0 0 8px 0;
  font-size: 32px;
  font-weight: 700;
  color: #1a202c;
}

.page-header p {
  margin: 0;
  color: #718096;
  font-size: 16px;
}

.search-section {
  background: white;
  border-radius: 12px;
  padding: 24px;
  box-shadow: 0 4px 6px rgba(0, 0, 0, 0.05);
  margin-bottom: 24px;
}

.search-row {
  display: flex;
  gap: 12px;
  margin-bottom: 16px;
}

.search-input {
  flex: 1;
  padding: 12px 16px;
  border: 2px solid #e2e8f0;
  border-radius: 8px;
  font-size: 16px;
  transition: border-color 0.2s;
}

.search-input:focus {
  outline: none;
  border-color: #667eea;
}

.search-btn {
  padding: 12px 24px;
  background: #3b82f6;
  color: white;
  border: none;
  border-radius: 8px;
  font-size: 16px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s;
}

.search-btn:hover:not(:disabled) {
  background: #2563eb;
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(59, 130, 246, 0.3);
}

.search-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.filter-options {
  display: flex;
  gap: 24px;
  align-items: center;
  flex-wrap: wrap;
}

.pages-selector {
  display: flex;
  align-items: center;
  gap: 8px;
}

.pages-selector select {
  padding: 8px 12px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  font-size: 14px;
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  font-size: 14px;
}

.checkbox-label input[type="checkbox"] {
  width: 16px;
  height: 16px;
}

.status {
  margin-top: 16px;
  padding: 12px;
  background: #f7fafc;
  border-radius: 6px;
  color: #4a5568;
  font-size: 14px;
}

.results-section {
  background: white;
  border-radius: 12px;
  padding: 24px;
  box-shadow: 0 4px 6px rgba(0, 0, 0, 0.05);
  width: 100%;
  overflow-x: hidden;
  min-width: 0;
}

.results-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 24px;
  padding-bottom: 16px;
  border-bottom: 1px solid #e2e8f0;
}

.results-header h2 {
  margin: 0;
  font-size: 24px;
  font-weight: 600;
  color: #1a202c;
}

.sort-controls {
  display: flex;
  align-items: center;
  gap: 8px;
}

.sort-selector {
  padding: 8px 12px;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  font-size: 14px;
}

.results-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: 20px;
  align-items: start;
  width: 100%;
  min-width: 0;
  overflow: hidden;
}

/* 响应式设计：在极小宽度时切换到单列 */
@media (max-width: 1200px) {
  .results-grid {
    gap: 16px;
  }
}

@media (max-width: 900px) {
  .results-grid {
    gap: 12px;
  }
}

@media (max-width: 700px) {
  .results-grid {
    grid-template-columns: 1fr;
    gap: 15px;
  }
}



@media (max-width: 600px) {
  .results-grid {
    gap: 12px;
  }
}

.result-item-wrapper {
  margin-bottom: 0;
}

.error-details {
  margin-top: 8px;
  padding: 12px;
  background: #fed7d7;
  border-radius: 6px;
  color: #c53030;
  font-size: 14px;
}

.error-header {
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 8px;
  user-select: none;
}

.error-header:hover {
  background: rgba(197, 48, 48, 0.1);
  border-radius: 4px;
  padding: 4px;
  margin: -4px;
}

.error-preview {
  flex: 1;
  font-weight: normal;
}

.error-toggle {
  font-size: 12px;
  color: #a53030;
  transition: transform 0.2s ease;
}

.error-full {
  margin-top: 8px;
  padding: 8px;
  background: rgba(197, 48, 48, 0.1);
  border-radius: 4px;
  font-size: 12px;
  line-height: 1.4;
  word-break: break-word;
  white-space: pre-wrap;
}
</style>
