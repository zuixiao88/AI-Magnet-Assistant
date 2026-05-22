<script setup lang="ts">
import { ref, computed, onMounted } from 'vue';
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from '../composables/useI18n';
import { logger } from '../utils/logger';

interface Props {
  title?: string;
  originalTitle?: string;
  magnetLink?: string;
  fileSize?: string;
  uploadDate?: string;
  analysis?: any;
  isPriority?: boolean;
  fileList?: string[];
  sourceEngine?: string;
  sourceUrl?: string;
  previewImageUrl?: string;
}

const props = defineProps<Props>();

const emit = defineEmits<{
  addToFavorites: [result: any];
  showNotification: [message: string, type?: 'success' | 'error'];
}>();

const { t } = useI18n();

const showFullLink = ref(false);
const copied = ref(false);
const quickDownloadEnabled = ref(true);
const isDownloading = ref(false);
const isPlaying = ref(false);
const showContentPreview = ref(false);
const previewImageFailed = ref(false);

const fileCount = computed(() => props.fileList?.length || 0);
const previewType = computed(() => {
  const text = `${props.title || ''} ${(props.fileList || []).join(' ')}`.toLowerCase();

  if (text.match(/\.(mp4|mkv|avi|mov|wmv|flv|webm)\b/) || text.includes('1080p') || text.includes('2160p')) {
    return { className: 'preview-video', icon: '🎬', label: 'VIDEO' };
  }
  if (text.match(/\.(mp3|flac|wav|aac|m4a)\b/)) {
    return { className: 'preview-audio', icon: '♪', label: 'AUDIO' };
  }
  if (text.match(/\.(zip|rar|7z|iso|dmg|exe|msi)\b/)) {
    return { className: 'preview-archive', icon: '⬡', label: 'FILE' };
  }
  if (text.match(/\.(pdf|epub|mobi|txt)\b/)) {
    return { className: 'preview-doc', icon: '▤', label: 'DOC' };
  }

  return { className: 'preview-generic', icon: '◇', label: 'BT' };
});
const magnetHash = computed(() => {
  if (!props.magnetLink) return '';

  const match = props.magnetLink.match(/btih:([^&]+)/i);
  return match?.[1] || '';
});
const magnetDisplayName = computed(() => getMagnetParam('dn'));
const trackerCount = computed(() => getMagnetParams('tr').length);

// 计算剩余文件的tooltip内容
const remainingFilesTooltip = computed(() => {
  if (!props.fileList || props.fileList.length <= 7) return '';
  const remainingFiles = props.fileList.slice(7);
  return remainingFiles.join('\n');
});

async function copyMagnetToClipboard(text: string | undefined, notifyOnFail = true): Promise<boolean> {
  if (!text) return false;
  try {
    await navigator.clipboard.writeText(text);
    copied.value = true;
    setTimeout(() => {
      copied.value = false;
    }, 2000);
    return true;
  } catch (error) {
    logger.error("Failed to copy magnet link:", error);
    if (notifyOnFail) {
      emit('showNotification', t('components.resultCard.messages.copyFailed'), 'error');
    }
    return false;
  }
}

async function copyToClipboard(text: string | undefined) {
  await copyMagnetToClipboard(text);
}

function toggleLinkDisplay() {
  showFullLink.value = !showFullLink.value;
}

function toggleContentPreview() {
  showContentPreview.value = !showContentPreview.value;
}

function handlePreviewImageError() {
  previewImageFailed.value = true;
}

function getDisplayLink(link: string | undefined) {
  if (!link) return '';
  if (showFullLink.value) return link;
  return link.length > 60 ? link.substring(0, 60) + '...' : link;
}

function getMagnetParams(name: string): string[] {
  if (!props.magnetLink) return [];

  try {
    const query = props.magnetLink.startsWith('magnet:?')
      ? props.magnetLink.slice('magnet:?'.length)
      : props.magnetLink;
    return new URLSearchParams(query).getAll(name).filter(Boolean);
  } catch {
    return [];
  }
}

function getMagnetParam(name: string): string {
  return getMagnetParams(name)[0] || '';
}

function addToFavorites() {
  const result = {
    title: props.title,
    magnet_link: props.magnetLink,
    file_size: props.fileSize,
    upload_date: props.uploadDate,
    file_list: props.fileList || [],
  };
  emit('addToFavorites', result);
}

function openSourceUrl() {
  if (props.sourceUrl) {
    window.open(props.sourceUrl, '_blank');
  }
}

// 快速下载相关函数
onMounted(async () => {
  await loadDownloadConfig();
});

async function loadDownloadConfig() {
  try {
    const config = await invoke("get_download_config");
    quickDownloadEnabled.value = (config as any).enable_quick_download;
  } catch (error) {
    logger.error("Failed to load download config:", error);
    quickDownloadEnabled.value = false;
  }
}

async function quickDownload(magnetLink: string | undefined) {
  if (!magnetLink) return;

  isDownloading.value = true;
  try {
    await invoke("open_magnet_link", { magnetLink });
    logger.debug("Magnet link opened successfully");
    emit('showNotification', t('components.resultCard.messages.downloadStarted'), 'success');
  } catch (error) {
    logger.error("Failed to open magnet link:", error);
    emit('showNotification', t('components.resultCard.messages.downloadFailed', { error: String(error) }), 'error');
  } finally {
    isDownloading.value = false;
  }
}

async function playMagnet(magnetLink: string | undefined) {
  if (!magnetLink) return;

  isPlaying.value = true;
  try {
    const copiedToClipboard = await copyMagnetToClipboard(magnetLink, false);
    await invoke("play_magnet_link", { magnetLink });
    emit(
      'showNotification',
      copiedToClipboard
        ? t('components.resultCard.messages.playStarted')
        : t('components.resultCard.messages.playStartedCopyFallback'),
      copiedToClipboard ? 'success' : 'error'
    );
  } catch (error) {
    logger.error("Failed to play magnet link:", error);
    emit('showNotification', t('components.resultCard.messages.playFailed', { error: String(error) }), 'error');
  } finally {
    isPlaying.value = false;
  }
}

</script>

<template>
  <div class="card">
    <div class="card-header">
      <div class="title-section">
        <div class="title-row">
          <div class="title-wrapper">
            <h3 class="title" :title="originalTitle || title">{{ title }}</h3>
            <span v-if="isPriority" class="priority-badge">📌 {{ $t('components.resultCard.priority') }}</span>
          </div>
          <div class="actions">
            <button v-if="sourceUrl" @click="openSourceUrl" class="action-btn" :title="$t('components.resultCard.actions.goToSource')">
              🔗
            </button>
            <button @click="addToFavorites" class="favorite-btn" :title="$t('components.resultCard.actions.addToFavorites')">
              ⭐
            </button>
          </div>
        </div>
        <div class="metadata-row" v-if="fileSize || uploadDate || sourceEngine || analysis">
          <div class="metadata-left">
            <span v-if="fileSize" class="file-size">📁 {{ fileSize }}</span>
            <span v-if="uploadDate" class="upload-date">📅 {{ uploadDate }}</span>
            <span v-if="sourceEngine" class="source-engine">🔎 {{ sourceEngine }}</span>
            <span v-if="analysis && analysis.purity_score" class="purity-score">
              🎯 {{ $t('components.resultCard.analysis.score') }}: {{ analysis.purity_score }}
            </span>
            <span v-if="analysis && analysis.tags && analysis.tags.length > 0" class="tags-item">
              🏷️ {{ analysis.tags.join(', ') }}
            </span>
          </div>
        </div>
      </div>
    </div>

    <!-- 文件列表预览区域 -->
    <div v-if="fileList && fileList.length > 0" class="file-list-section">
      <div class="file-grid">
        <div
          v-for="file in fileList.slice(0, 7)"
          :key="file"
          class="file-item"
          :title="file"
        >
          <div class="file-icon">📄</div>
          <div class="file-name">{{ file }}</div>
        </div>
        <div v-if="fileList.length > 7" class="more-files" :title="remainingFilesTooltip">
          {{ $t('components.resultCard.fileList.moreFiles', { count: fileList.length - 7 }) }}
        </div>
      </div>
    </div>

    <div class="magnet-section">
      <div class="magnet-link-container">
        <div class="magnet-link" @click="toggleLinkDisplay">
          <code>{{ getDisplayLink(magnetLink) }}</code>
          <span v-if="magnetLink && magnetLink.length > 60" class="toggle-hint">
            {{ showFullLink ? $t('components.resultCard.fileList.collapseHint') : $t('components.resultCard.fileList.expandHint') }}
          </span>
        </div>
        <button
          @click="copyToClipboard(magnetLink)"
          class="copy-btn-icon"
          :class="{ 'copied': copied }"
          :title="copied ? $t('components.resultCard.actions.copied') : $t('components.resultCard.actions.copyMagnetLink')"
        >
          {{ copied ? '✓' : '📋' }}
        </button>
        <button
          v-if="quickDownloadEnabled"
          @click="quickDownload(magnetLink)"
          class="quick-download-btn"
          :class="{ 'downloading': isDownloading }"
          :title="isDownloading ? $t('components.resultCard.actions.opening') : $t('components.resultCard.actions.quickDownload')"
          :disabled="isDownloading"
        >
          {{ isDownloading ? '⏳' : '⬇️' }}
        </button>
        <button
          @click="playMagnet(magnetLink)"
          class="play-btn"
          :class="{ playing: isPlaying }"
          :title="isPlaying ? $t('components.resultCard.actions.opening') : $t('components.resultCard.actions.playMagnet')"
          :disabled="isPlaying"
        >
          {{ isPlaying ? '⏳' : '▶' }}
        </button>
        <button
          @click="toggleContentPreview"
          class="preview-btn"
          :class="{ active: showContentPreview }"
          :title="$t('components.resultCard.actions.previewContent')"
        >
          {{ showContentPreview ? '▴' : '☰' }}
        </button>
      </div>
    </div>

    <div v-if="showContentPreview" class="content-preview">
      <div class="preview-header">
        <div>
          <h4>{{ $t('components.resultCard.preview.title') }}</h4>
          <p>{{ $t('components.resultCard.preview.subtitle') }}</p>
        </div>
        <button
          @click="toggleContentPreview"
          class="preview-close-btn"
          :title="$t('components.resultCard.preview.close')"
        >
          ×
        </button>
      </div>

      <div class="preview-media-section">
        <div class="preview-media-frame" :class="previewType.className">
          <img
            v-if="previewImageUrl && !previewImageFailed"
            :src="previewImageUrl"
            :alt="title || 'preview'"
            loading="lazy"
            @error="handlePreviewImageError"
          />
          <template v-else>
            <span class="preview-thumb-icon">{{ previewType.icon }}</span>
            <span class="preview-thumb-label">{{ previewType.label }}</span>
          </template>
        </div>
        <div class="preview-media-copy">
          <div class="preview-section-title">{{ $t('components.resultCard.preview.previewImage') }}</div>
          <div class="preview-media-caption">
            {{ previewImageUrl && !previewImageFailed ? $t('components.resultCard.preview.previewImageLoaded') : $t('components.resultCard.preview.previewImageFallback') }}
          </div>
        </div>
      </div>

      <div class="preview-meta">
        <div class="preview-meta-item">
          <span>{{ $t('components.resultCard.preview.fileCount') }}</span>
          <strong>{{ fileCount }}</strong>
        </div>
        <div class="preview-meta-item">
          <span>{{ $t('components.resultCard.preview.fileSize') }}</span>
          <strong>{{ fileSize || $t('components.resultCard.preview.unknown') }}</strong>
        </div>
        <div class="preview-meta-item preview-hash" v-if="magnetHash">
          <span>{{ $t('components.resultCard.preview.hash') }}</span>
          <strong>{{ magnetHash }}</strong>
        </div>
        <div class="preview-meta-item" v-if="magnetDisplayName">
          <span>{{ $t('components.resultCard.preview.displayName') }}</span>
          <strong>{{ magnetDisplayName }}</strong>
        </div>
        <div class="preview-meta-item">
          <span>{{ $t('components.resultCard.preview.trackers') }}</span>
          <strong>{{ trackerCount }}</strong>
        </div>
      </div>

      <div class="preview-section">
        <div class="preview-section-title">{{ $t('components.resultCard.preview.magnetLink') }}</div>
        <code class="preview-magnet">{{ magnetLink }}</code>
      </div>

      <div class="preview-section">
        <div class="preview-section-title">{{ $t('components.resultCard.preview.files') }}</div>
        <div v-if="fileList && fileList.length > 0" class="preview-file-list">
          <div v-for="(file, index) in fileList" :key="`${file}-${index}`" class="preview-file-row">
            <span class="preview-file-index">{{ index + 1 }}</span>
            <span class="preview-file-name">{{ file }}</span>
          </div>
        </div>
        <div v-else class="preview-empty">
          {{ $t('components.resultCard.preview.noFiles') }}
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.card {
  border: 1px solid #e0e0e0;
  border-radius: 12px;
  padding: 16px;
  background: white;
  box-shadow: 0 2px 8px rgba(0,0,0,0.1);
  transition: box-shadow 0.3s ease;
  height: 100%;
  display: flex;
  flex-direction: column;
  width: 100%;
  box-sizing: border-box;
  min-width: 0;
  overflow: hidden;
}

.card:hover {
  box-shadow: 0 4px 12px rgba(0,0,0,0.15);
}

.card-header {
  margin-bottom: 10px;
  display: block;
}

.preview-media-section {
  display: grid;
  grid-template-columns: 132px minmax(0, 1fr);
  gap: 12px;
  align-items: center;
  padding: 12px 14px;
}

.preview-media-frame {
  width: 132px;
  aspect-ratio: 16 / 10;
  border-radius: 8px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-items: center;
  border: 1px solid #d8dee8;
}

.preview-media-frame img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.preview-media-copy {
  min-width: 0;
}

.preview-media-caption {
  color: #64748b;
  font-size: 12px;
  line-height: 1.5;
}

.preview-thumb {
  width: 72px;
  aspect-ratio: 1;
  border-radius: 8px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-items: center;
  border: 1px solid #d8dee8;
  flex-shrink: 0;
}

.preview-thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.preview-thumb-icon {
  font-size: 24px;
  line-height: 1;
  color: #1f2937;
}

.preview-thumb-label {
  margin-top: 6px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0;
  color: #4b5563;
}

.preview-video {
  background: linear-gradient(135deg, #e8f4ff, #f7fbff);
}

.preview-audio {
  background: linear-gradient(135deg, #f0fdf4, #f8fff9);
}

.preview-archive {
  background: linear-gradient(135deg, #fff7ed, #fffaf4);
}

.preview-doc {
  background: linear-gradient(135deg, #eef2ff, #fafbff);
}

.preview-generic {
  background: linear-gradient(135deg, #f8fafc, #ffffff);
}

.title-section {
  display: flex;
  flex-direction: column;
  width: 100%;
}

.title-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  width: 100%;
}

.title-wrapper {
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0; /* Allow shrinking */
  flex: 1; /* Allow growing */
  overflow: hidden; /* Prevent content from overflowing */
}

.title {
  margin: 0;
  color: #2c3e50;
  font-size: 1.1em;
  font-weight: 600;
  line-height: 1.4;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.priority-badge {
  background: #3b82f6;
  color: white;
  padding: 4px 8px;
  border-radius: 12px;
  font-size: 11px;
  font-weight: 500;
  white-space: nowrap;
  flex-shrink: 0;
}

.metadata-row {
  margin-top: 8px;
  width: 100%;
}

.metadata-left {
  display: flex;
  flex-wrap: wrap;
  gap: 15px;
  font-size: 12px;
  color: #7f8c8d;
  align-items: center;
}

.file-size, .upload-date, .purity-score, .source-engine {
  display: flex;
  align-items: center;
  gap: 4px;
  white-space: nowrap;
  flex-shrink: 0;
}

.source-engine {
  color: #475569;
  font-weight: 600;
}

.purity-score {
  color: #27ae60;
  font-weight: 600;
}

.tags-item {
  display: flex;
  align-items: center;
  gap: 4px;
  color: #3498db;
  font-style: italic;
  flex: 1;
  min-width: 0;
  line-height: 1.4;
}

.actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.action-btn, .favorite-btn, .copy-btn-icon, .preview-btn, .play-btn {
  border: none;
  cursor: pointer;
  transition: all 0.3s ease;
  white-space: nowrap;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  padding: 4px;
  border-radius: 50%;
}

.action-btn:hover, .favorite-btn:hover, .copy-btn-icon:hover, .preview-btn:hover, .play-btn:hover:not(:disabled) {
  background: #f0f0f0;
}

.action-btn {
  font-size: 18px;
  color: #3498db;
}

.copy-btn-icon {
  font-size: 18px;
  color: #3498db;
  flex-shrink: 0;
}

.copy-btn-icon.copied {
  color: #27ae60;
}

.quick-download-btn {
  border: none;
  cursor: pointer;
  transition: all 0.3s ease;
  white-space: nowrap;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  padding: 4px;
  border-radius: 50%;
  font-size: 18px;
  font-weight: 900;
  color: #1e3a8a;
  flex-shrink: 0;
}

.quick-download-btn:hover:not(:disabled) {
  background: #f0f0f0;
  transform: scale(1.1);
}

.quick-download-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.quick-download-btn.downloading {
  color: #f39c12;
  animation: pulse 1.5s infinite;
}

.play-btn {
  color: #16a34a;
  font-size: 16px;
  flex-shrink: 0;
}

.play-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.play-btn.playing {
  color: #f39c12;
  animation: pulse 1.5s infinite;
}

.preview-btn {
  color: #475569;
  font-size: 17px;
  flex-shrink: 0;
}

.preview-btn.active {
  color: #2563eb;
  background: #e0ecff;
}

@keyframes pulse {
  0% { opacity: 1; }
  50% { opacity: 0.5; }
  100% { opacity: 1; }
}

.favorite-btn {
  background: transparent;
  color: #ffc107;
  font-size: 20px;
  padding: 4px;
  border-radius: 50%;
}

.favorite-btn:hover {
  background: #f0f0f0;
}

.file-list-section {
  margin-top: 5px;
  padding-top: 5px;
  width: 100%;
  box-sizing: border-box;
}

.file-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: 6px;
  width: 100%;
  min-width: 0;
  overflow-x: hidden;
}

.file-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  background: #f8fafc;
  border: 1px solid #e2e8f0;
  border-radius: 4px;
  transition: all 0.2s;
  cursor: pointer;
  width: 100%;
  box-sizing: border-box;
  min-width: 0;
  overflow: hidden;
  word-break: break-all;
}

.file-item:hover {
  background: #f1f5f9;
  border-color: #cbd5e0;
  transform: translateY(-1px);
  box-shadow: 0 2px 4px rgba(0, 0, 0, 0.05);
}

.file-icon {
  font-size: 14px;
  flex-shrink: 0;
}

.file-name {
  font-size: 12px;
  color: #4a5568;
  font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
  line-height: 1.3;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}

.more-files {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  background: #f1f5f9;
  border: 1px solid #cbd5e0;
  border-radius: 4px;
  transition: all 0.2s;
  cursor: pointer;
  width: 100%;
  box-sizing: border-box;
  min-width: 0;
  overflow: hidden;
  word-break: break-all;
  font-size: 12px;
  color: #64748b;
  font-weight: 500;
  position: relative;
}

.more-files:hover {
  background: #e2e8f0;
  border-color: #a0aec0;
  transform: translateY(-1px);
  box-shadow: 0 2px 4px rgba(0, 0, 0, 0.05);
}

.more-files::before {
  content: "📁";
  font-size: 14px;
  flex-shrink: 0;
}

.magnet-section {
  margin-top: 15px;
  flex-grow: 1;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
}

.magnet-link-container {
  display: flex;
  align-items: center;
  gap: 8px;
  background: #f8f9fa;
  border: 1px solid #e9ecef;
  border-radius: 6px;
  padding: 10px;
}

.magnet-link {
  flex-grow: 1;
  cursor: pointer;
  min-width: 0;
}

.magnet-link:hover {
  background: #e9ecef;
}

.magnet-link code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 11px;
  color: #2c3e50;
  word-break: break-all;
  display: block;
  overflow-wrap: break-word;
  min-width: 0;
}

.content-preview {
  margin-top: 12px;
  border: 1px solid #d8e2f0;
  border-radius: 8px;
  background: #fbfdff;
  overflow: hidden;
}

.preview-header {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  background: #eef5ff;
  border-bottom: 1px solid #d8e2f0;
}

.preview-header h4 {
  margin: 0;
  font-size: 14px;
  color: #1e3a5f;
}

.preview-header p {
  margin: 4px 0 0;
  font-size: 12px;
  color: #64748b;
}

.preview-close-btn {
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: #64748b;
  font-size: 20px;
  line-height: 1;
  cursor: pointer;
}

.preview-close-btn:hover {
  background: #dbeafe;
  color: #1e40af;
}

.preview-meta {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
  padding: 12px 14px;
}

.preview-meta-item {
  min-width: 0;
  padding: 8px 10px;
  background: white;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
}

.preview-meta-item span {
  display: block;
  margin-bottom: 4px;
  font-size: 11px;
  color: #64748b;
}

.preview-meta-item strong {
  display: block;
  font-size: 12px;
  color: #1f2937;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preview-hash {
  grid-column: span 1;
}

.preview-section {
  padding: 0 14px 12px;
}

.preview-section-title {
  margin-bottom: 6px;
  font-size: 12px;
  font-weight: 700;
  color: #334155;
}

.preview-magnet {
  display: block;
  max-height: 78px;
  overflow: auto;
  padding: 8px 10px;
  background: #0f172a;
  color: #dbeafe;
  border-radius: 6px;
  font-size: 11px;
  line-height: 1.45;
  word-break: break-all;
}

.preview-file-list {
  max-height: 220px;
  overflow: auto;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  background: white;
}

.preview-file-row {
  display: grid;
  grid-template-columns: 32px minmax(0, 1fr);
  gap: 8px;
  padding: 7px 10px;
  border-bottom: 1px solid #f1f5f9;
}

.preview-file-row:last-child {
  border-bottom: none;
}

.preview-file-index {
  color: #94a3b8;
  font-size: 12px;
  text-align: right;
}

.preview-file-name {
  min-width: 0;
  color: #334155;
  font-size: 12px;
  overflow-wrap: anywhere;
}

.preview-empty {
  padding: 12px;
  background: white;
  border: 1px solid #e2e8f0;
  border-radius: 6px;
  color: #64748b;
  font-size: 12px;
}

.toggle-hint {
  display: block;
  font-size: 11px;
  color: #95a5a6;
  margin-top: 5px;
  font-style: italic;
}

/* 响应式设计 */
@media (max-width: 1200px) {
  .card { padding: 12px; }
  .file-grid { gap: 4px; }
  .file-item { padding: 4px 6px; gap: 4px; }
  .file-list-section { margin-top: 5px; padding-top: 5px; }
}

@media (max-width: 900px) {
  .card { padding: 10px; }
  .file-item { padding: 3px 5px; gap: 3px; }
  .file-name { font-size: 11px; }
  .file-icon { font-size: 10px; }
}

@media (max-width: 600px) {
  .card { padding: 8px; }
  .card-header { margin-bottom: 8px; }
  .preview-thumb { width: 56px; }
  .preview-media-section { grid-template-columns: 1fr; padding: 10px 12px; }
  .preview-media-frame { width: 100%; }
  .preview-thumb-icon { font-size: 20px; }
  .preview-thumb-label { font-size: 9px; }
  .title { font-size: 1em; }
  .file-grid { gap: 3px; max-height: 100px; }
  .file-item { padding: 2px 4px; gap: 2px; }
  .file-name { font-size: 10px; }
  .file-icon { font-size: 9px; }
  .file-list-section { margin-top: 5px; padding-top: 5px; }
  .magnet-link-container { padding: 6px; }
  .magnet-link code { font-size: 10px; }
  .metadata { font-size: 11px; }
  .preview-meta { grid-template-columns: 1fr; }
}

@media (max-width: 400px) {
  .card { padding: 6px; }
  .card-header { margin-bottom: 6px; }
  .preview-thumb { width: 48px; }
  .title { font-size: 0.9em; }
  .file-grid { gap: 2px; max-height: 80px; }
  .file-item { padding: 1px 3px; gap: 1px; }
  .file-name { font-size: 9px; }
  .file-icon { font-size: 8px; }
  .magnet-link-container { padding: 4px; }
  .magnet-link code { font-size: 9px; }
}
</style>
