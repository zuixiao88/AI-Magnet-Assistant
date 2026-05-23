<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useI18n } from '../composables/useI18n';
import { logger } from '../utils/logger';

const props = defineProps<{
  title?: string;
  magnetLink: string;
}>();

const emit = defineEmits<{
  close: [];
  showNotification: [message: string, type?: 'success' | 'error'];
}>();

const { t } = useI18n();

type PlayerSession = {
  base_url: string;
  info_hash: string;
};

type PlayerFile = {
  name: string;
  url: string;
  length?: number | null;
  media_type: 'video' | 'audio' | string;
};

const files = ref<PlayerFile[]>([]);
const status = ref(t('components.resultCard.player.status.loadingEngine'));
const warning = ref('');
const selectedFileName = ref('');
const selectedFileUrl = ref('');
const selectedMediaType = ref('video');
const progress = ref('0.0%');
const downloadSpeed = ref('0 B/s');
const peers = ref('0');
const isReady = ref(false);
const hasError = ref(false);

let session: PlayerSession | null = null;
let playlistTimer: number | null = null;
let statsTimer: number | null = null;

const playableFiles = computed(() => files.value.filter(isNativePlayableFile));
const selectedFile = computed(() => files.value.find(file => file.url === selectedFileUrl.value || file.name === selectedFileName.value));
const canRenderSelected = computed(() => !!selectedFile.value && isNativePlayableFile(selectedFile.value));

function isNativePlayableFile(file: PlayerFile) {
  const lower = file.name.toLowerCase();
  return /\.(mp4|m4v|mov|webm|mp3|ogg|wav|aac|m4a)$/i.test(lower);
}

function formatBytes(bytes?: number | null) {
  if (!bytes || bytes < 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / Math.pow(1024, index)).toFixed(index ? 1 : 0)} ${units[index]}`;
}

function renderFile(file: PlayerFile) {
  if (!isNativePlayableFile(file)) {
    hasError.value = false;
    selectedFileName.value = file.name;
    selectedFileUrl.value = file.url;
    selectedMediaType.value = file.media_type === 'audio' ? 'audio' : 'video';
    status.value = t('components.resultCard.player.status.unsupportedFormat');
    return;
  }

  hasError.value = false;
  selectedFileName.value = file.name;
  selectedFileUrl.value = file.url;
  selectedMediaType.value = file.media_type === 'audio' ? 'audio' : 'video';
  status.value = t('components.resultCard.player.status.preparing');
}

function handlePlaybackReady() {
  status.value = t('components.resultCard.player.status.playing');
}

function handlePlaybackError(event: Event) {
  const target = event.target as HTMLMediaElement;
  const message = target.error?.message || target.error?.code || 'unsupported media stream';
  hasError.value = true;
  status.value = t('components.resultCard.player.status.renderFailed', { error: String(message) });
}

async function openSelectedStream() {
  if (!selectedFileUrl.value) return;

  try {
    await invoke('open_stream_url', { url: selectedFileUrl.value });
  } catch (error) {
    logger.error('Failed to open stream URL:', error);
    emit('showNotification', t('components.resultCard.player.status.openStreamFailed', { error: String(error) }), 'error');
  }
}

async function refreshPlaylist() {
  if (!session) return;

  try {
    const nextFiles = await invoke<PlayerFile[]>('get_builtin_magnet_playlist', {
      infoHash: session.info_hash,
      baseUrl: session.base_url,
    });

    files.value = nextFiles;
    if (nextFiles.length > 0) {
      isReady.value = true;
      status.value = selectedFileName.value
        ? status.value
        : t('components.resultCard.player.status.loaded', { name: props.title || session.info_hash });
      const firstPlayable = playableFiles.value[0];
      if (!selectedFileName.value && firstPlayable) {
        renderFile(firstPlayable);
      } else if (!selectedFileName.value && nextFiles.length > 0) {
        selectedFileName.value = nextFiles[0].name;
        selectedFileUrl.value = nextFiles[0].url;
        selectedMediaType.value = nextFiles[0].media_type === 'audio' ? 'audio' : 'video';
        status.value = t('components.resultCard.player.status.noNativePlayableFile');
      }
    }
  } catch (error) {
    logger.warn('Failed to refresh built-in magnet playlist:', error);
  }
}

function startTimers() {
  if (playlistTimer) {
    window.clearInterval(playlistTimer);
  }
  if (statsTimer) {
    window.clearInterval(statsTimer);
  }

  playlistTimer = window.setInterval(refreshPlaylist, 2500);
  statsTimer = window.setInterval(async () => {
    if (!session) return;

    try {
      const stats = await invoke<{ progress?: number; download_speed?: number; peers?: number }>('get_builtin_magnet_stats', {
        infoHash: session.info_hash,
        baseUrl: session.base_url,
      });

      progress.value = `${((stats.progress || 0) * 100).toFixed(1)}%`;
      downloadSpeed.value = `${formatBytes(stats.download_speed || 0)}/s`;
      peers.value = String(stats.peers || 0);
    } catch (error) {
      logger.warn('Failed to refresh built-in magnet stats:', error);
    }
  }, 1000);
}

async function startPlayer() {
  try {
    status.value = t('components.resultCard.player.status.loadingEngine');
    warning.value = t('components.resultCard.player.status.localEngineHint');

    session = await invoke<PlayerSession>('start_builtin_magnet_player', {
      magnetLink: props.magnetLink,
    });

    status.value = t('components.resultCard.player.status.connecting');
    await refreshPlaylist();
    startTimers();
  } catch (error) {
    logger.error('Failed to start built-in magnet player:', error);
    hasError.value = true;
    status.value = t('components.resultCard.player.status.startFailed', { error: String(error) });
    emit('showNotification', status.value, 'error');
  }
}

function closePlayer() {
  emit('close');
}

onMounted(startPlayer);

onBeforeUnmount(() => {
  if (playlistTimer) {
    window.clearInterval(playlistTimer);
    playlistTimer = null;
  }
  if (statsTimer) {
    window.clearInterval(statsTimer);
    statsTimer = null;
  }
});
</script>

<template>
  <div class="player-overlay" @click.self="closePlayer">
    <div class="player-dialog">
      <div class="player-header">
        <div class="player-title">
          <h4>{{ $t('components.resultCard.player.title') }}</h4>
          <p>{{ title }}</p>
        </div>
        <button class="player-close-btn" @click="closePlayer" :title="$t('components.resultCard.player.close')">×</button>
      </div>

      <div class="player-body">
        <div class="player-main">
          <div class="player-stage" :class="{ empty: !selectedFileName, error: hasError }">
            <video
              v-if="selectedFileUrl && canRenderSelected && selectedMediaType === 'video'"
              :key="selectedFileUrl"
              :src="selectedFileUrl"
              controls
              autoplay
              playsinline
              @canplay="handlePlaybackReady"
              @error="handlePlaybackError"
            />
            <audio
              v-else-if="selectedFileUrl && canRenderSelected && selectedMediaType === 'audio'"
              :key="selectedFileUrl"
              :src="selectedFileUrl"
              controls
              autoplay
              @canplay="handlePlaybackReady"
              @error="handlePlaybackError"
            />
            <span v-else>{{ status }}</span>
          </div>
          <div class="player-status">
            <span>{{ status }}</span>
            <button v-if="selectedFileUrl" class="stream-open-btn" @click="openSelectedStream">
              {{ $t('components.resultCard.player.openStream') }}
            </button>
            <small v-if="warning">{{ warning }}</small>
          </div>
        </div>

        <aside class="player-side">
          <div class="player-stats">
            <div><span>{{ $t('components.resultCard.player.stats.progress') }}</span><strong>{{ progress }}</strong></div>
            <div><span>{{ $t('components.resultCard.player.stats.speed') }}</span><strong>{{ downloadSpeed }}</strong></div>
            <div><span>{{ $t('components.resultCard.player.stats.peers') }}</span><strong>{{ peers }}</strong></div>
          </div>

          <div class="player-files">
            <h5>{{ $t('components.resultCard.player.files') }}</h5>
            <button
              v-for="file in files"
              :key="file.name"
              class="player-file"
              :class="{ active: selectedFileName === file.name, playable: isNativePlayableFile(file) }"
              @click="renderFile(file)"
              :title="file.name"
            >
              <span>{{ file.name }}</span>
              <small>{{ formatBytes(file.length) }}</small>
            </button>
            <div v-if="isReady && files.length === 0" class="player-empty">
              {{ $t('components.resultCard.player.status.noFiles') }}
            </div>
          </div>
        </aside>
      </div>
    </div>
  </div>
</template>

<style scoped>
.player-overlay {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  background: rgba(15, 23, 42, 0.72);
}

.player-dialog {
  width: min(1180px, 96vw);
  height: min(780px, 90vh);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border-radius: 8px;
  background: #0f172a;
  box-shadow: 0 24px 64px rgba(0, 0, 0, 0.34);
}

.player-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
  padding: 12px 14px;
  border-bottom: 1px solid #263449;
  color: #e5eefb;
}

.player-title {
  min-width: 0;
}

.player-title h4 {
  margin: 0;
  font-size: 15px;
}

.player-title p {
  margin: 4px 0 0;
  max-width: 820px;
  overflow: hidden;
  color: #9fb2ce;
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.player-close-btn {
  width: 30px;
  height: 30px;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: #cbd5e1;
  font-size: 24px;
  line-height: 1;
  cursor: pointer;
}

.player-close-btn:hover {
  background: #1e293b;
  color: white;
}

.player-body {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) 320px;
}

.player-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  background: #020617;
}

.player-stage {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  color: #9fb2ce;
  text-align: center;
}

.player-stage :deep(video),
.player-stage :deep(audio) {
  width: 100%;
  max-height: 100%;
  background: #000;
}

.player-stage.error {
  color: #fecaca;
}

.player-status {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 4px;
  align-items: center;
  padding: 10px 12px;
  border-top: 1px solid #172033;
  color: #cbd5e1;
  font-size: 12px;
}

.stream-open-btn {
  padding: 6px 10px;
  border: 1px solid #3b82f6;
  border-radius: 6px;
  background: #1d4ed8;
  color: #eff6ff;
  font-size: 12px;
  cursor: pointer;
}

.stream-open-btn:hover {
  background: #2563eb;
}

.player-status small {
  grid-column: 1 / -1;
  color: #facc15;
  line-height: 1.4;
}

.player-side {
  min-width: 0;
  display: flex;
  flex-direction: column;
  border-left: 1px solid #263449;
  background: #111c30;
}

.player-stats {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
  padding: 10px;
  border-bottom: 1px solid #263449;
}

.player-stats div {
  min-width: 0;
  padding: 8px;
  border: 1px solid #263449;
  border-radius: 6px;
  background: #0c1525;
}

.player-stats span,
.player-stats strong {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.player-stats span {
  margin-bottom: 3px;
  color: #8aa0bd;
  font-size: 11px;
}

.player-stats strong {
  color: #e5eefb;
  font-size: 12px;
}

.player-files {
  min-height: 0;
  overflow: auto;
  padding: 10px;
}

.player-files h5 {
  margin: 0 0 8px;
  color: #e5eefb;
  font-size: 13px;
}

.player-file {
  width: 100%;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 8px;
  align-items: center;
  padding: 8px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: #dbeafe;
  text-align: left;
  cursor: pointer;
}

.player-file:hover:not(:disabled),
.player-file.active {
  border-color: #3b82f6;
  background: #16243a;
}

.player-file span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.player-file small {
  color: #8aa0bd;
  white-space: nowrap;
}

.player-empty {
  color: #94a3b8;
  font-size: 12px;
}

@media (max-width: 860px) {
  .player-overlay {
    padding: 8px;
  }

  .player-dialog {
    width: 100%;
    height: 92vh;
  }

  .player-body {
    grid-template-columns: 1fr;
    grid-template-rows: minmax(0, 1fr) 260px;
  }

  .player-side {
    border-left: none;
    border-top: 1px solid #263449;
  }

  .player-title p {
    max-width: 260px;
  }
}
</style>
