<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { Terminal, RotateCw, FolderOpen, TriangleAlert, CheckCircle2, Download } from "lucide-vue-next";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  getStatus,
  readLog,
  restartDsh,
  openLogDir,
  checkUpdate,
  updateDsh,
  navigateWorkbench,
  onDshUpdate,
  type StatusInfo,
  type DshState,
  type UpdateInfo,
} from "../api/tauri";

const status = ref<StatusInfo>({ state: "starting" });
const log = ref<string>("");
const showLog = ref(false);
const loadingLog = ref(false);
const retrying = ref(false);
const updateInfo = ref<UpdateInfo | null>(null);
const updating = ref(false);
const updateMsg = ref("");
const updateError = ref("");
const skipped = ref(false);
/** 版本检查是否已完成（成功或失败都算），完成且就绪后才跳转工作台 */
const updateResolved = ref(false);
/** 是否已触发跳转，避免重复调用 */
const navigating = ref(false);
let timer: ReturnType<typeof setInterval> | null = null;
let navTimer: ReturnType<typeof setTimeout> | null = null;
let unlistenUpdate: UnlistenFn | null = null;

const phases: Record<DshState, string> = {
  starting: "正在启动 dsh 服务 · 正在连接本地运行时",
  ready: "dsh 服务已就绪",
  failed: "dsh 服务启动失败",
};

const isReady = computed(() => status.value.state === "ready");
const isFailed = computed(() => status.value.state === "failed");
const isStarting = computed(() => status.value.state === "starting");

const phaseText = computed(() => {
  if (status.value.state === "starting" && status.value.stage === "deploying") {
    return "正在部署运行时组件 · 首次启动需要几分钟";
  }
  return phases[status.value.state] ?? phases.starting;
});
const errorText = computed(() => status.value.error ?? "");

const accent = computed(() => {
  if (isReady.value) return "#10B981";
  if (isFailed.value) return "#EF4444";
  return "#22D3EE";
});

async function poll() {
  status.value = await getStatus();
  if (isFailed.value && log.value === "") {
    void loadLog();
  }
  maybeNavigate();
}

/** dsh 就绪后切换到工作台；版本检查未完成时最多再等 3.5s */
function maybeNavigate() {
  if (navigating.value || !isReady.value) return;
  if (updateResolved.value) {
    navigating.value = true;
    void navigateWorkbench().catch((err) => {
      console.error("跳转工作台失败：", err);
      navigating.value = false;
    });
    return;
  }
  if (!navTimer) {
    navTimer = setTimeout(() => {
      updateResolved.value = true;
      maybeNavigate();
    }, 3500);
  }
}

async function loadLog() {
  loadingLog.value = true;
  try {
    log.value = await readLog(200);
  } catch (err) {
    log.value = `读取日志失败：${String(err)}`;
    console.error(err);
  } finally {
    loadingLog.value = false;
  }
}

async function onRetry() {
  retrying.value = true;
  log.value = "";
  showLog.value = false;
  try {
    await restartDsh();
  } catch (err) {
    console.error(err);
  } finally {
    retrying.value = false;
  }
  await poll();
}

async function checkForUpdate() {
  try {
    updateInfo.value = await checkUpdate();
  } catch (err) {
    console.error(err);
  } finally {
    updateResolved.value = true;
    maybeNavigate();
  }
}

async function onUpdate() {
  updating.value = true;
  updateError.value = "";
  updateMsg.value = "正在准备更新…";
  try {
    await updateDsh();
  } catch (err) {
    updating.value = false;
    updateError.value = String(err);
  }
}

onMounted(async () => {
  void poll();
  timer = setInterval(() => void poll(), 1200);
  unlistenUpdate = await onDshUpdate((p) => {
    if (p.kind === "started") {
      updating.value = true;
      updateMsg.value = p.message;
    } else if (p.kind === "success") {
      updating.value = false;
      updateMsg.value = p.message;
      skipped.value = false;
      void checkForUpdate();
    } else if (p.kind === "error") {
      updating.value = false;
      updateError.value = p.message;
    }
  });
  void checkForUpdate();
});

onUnmounted(() => {
  if (timer) clearInterval(timer);
  if (navTimer) clearTimeout(navTimer);
  unlistenUpdate?.();
});
</script>

<template>
  <div
    class="relative flex h-full w-full items-center justify-center overflow-hidden"
    style="background: radial-gradient(1200px 600px at 50% -10%, rgba(14, 165, 233, 0.16), transparent 60%), radial-gradient(900px 500px at 85% 110%, rgba(139, 92, 246, 0.14), transparent 55%), #0b0f1a"
  >
    <!-- 网格背景 -->
    <div
      class="pointer-events-none absolute inset-0 opacity-[0.05]"
      style="background-image: linear-gradient(rgba(148, 163, 184, 0.6) 1px, transparent 1px), linear-gradient(90deg, rgba(148, 163, 184, 0.6) 1px, transparent 1px); background-size: 44px 44px"
    />

    <div class="glass relative w-[520px] max-w-[92vw] rounded-2xl p-10 animate-fade-up">
      <!-- Logo -->
      <div class="mb-8 flex flex-col items-center gap-5">
        <div
          class="flex h-16 w-16 items-center justify-center rounded-2xl"
          :style="{
            background: 'linear-gradient(135deg, rgba(34,211,238,0.18), rgba(59,130,246,0.12))',
            border: '1px solid rgba(34,211,238,0.3)',
            boxShadow: `0 0 40px ${accent}22`,
          }"
        >
          <Terminal :size="30" :color="accent" stroke-width="2.2" />
        </div>
        <div class="text-center">
          <h1 class="text-[22px] font-semibold tracking-wide">DeepSeek Harness</h1>
          <p class="mt-1 text-[13px] text-slate-400">Desktop Launcher</p>
        </div>
      </div>

      <!-- 状态区 -->
      <div class="flex items-center justify-center gap-3">
        <!-- 状态指示灯 -->
        <span class="relative flex h-3 w-3">
          <span
            v-if="isStarting"
            class="absolute inline-flex h-full w-full animate-ping rounded-full opacity-60"
            :style="{ background: accent }"
          />
          <span
            class="relative inline-flex h-3 w-3 rounded-full"
            :style="{ background: accent, boxShadow: `0 0 12px ${accent}` }"
          />
        </span>
        <span class="text-[14px] font-medium text-slate-200">{{ phaseText }}</span>
      </div>

      <div v-if="isStarting" class="mt-6">
        <div class="h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
          <div
            class="h-full rounded-full"
            style="background: linear-gradient(90deg, #22d3ee, #3b82f6); width: 30%; animation: loadbar 1.4s ease-in-out infinite"
          />
        </div>
      </div>

      <div v-if="isReady" class="mt-6 flex flex-col items-center gap-2">
        <div class="flex items-center gap-2 text-[13px] text-emerald-400">
          <CheckCircle2 :size="16" />
          <span>服务已就绪，正在打开工作台…</span>
        </div>
        <p v-if="status.port" class="font-mono text-[12px] text-slate-500">
          http://127.0.0.1:{{ status.port }}
        </p>
      </div>

      <!-- 版本与更新 -->
      <div v-if="updateInfo" class="mt-5 flex flex-col items-center gap-2">
        <p class="font-mono text-[12px] text-slate-500">
          dsh {{ updateInfo.current }}
          <span v-if="!updateInfo.has_update && !updateInfo.error" class="text-slate-600">（已是最新）</span>
        </p>

        <!-- 发现新版本 -->
        <div
          v-if="updateInfo.has_update && !skipped"
          class="mt-1 w-full animate-fade-up rounded-xl border border-amber-400/30 bg-amber-400/10 p-3 text-center"
        >
          <p class="text-[13px] text-amber-300">
            发现新版本 <span class="font-mono">{{ updateInfo.latest }}</span>
          </p>
          <div class="mt-2.5 flex justify-center gap-2">
            <button
              @click="onUpdate"
              :disabled="updating"
              class="flex items-center gap-1.5 rounded-lg border border-amber-400/50 bg-amber-400/15 px-3.5 py-1.5 text-[12px] font-medium text-amber-200 transition hover:bg-amber-400/25 disabled:opacity-60"
            >
              <Download :size="14" :class="{ 'animate-spin': updating }" />
              {{ updating ? "更新中…" : "立即更新并重启" }}
            </button>
            <button
              @click="skipped = true"
              :disabled="updating"
              class="rounded-lg border border-slate-600/50 px-3.5 py-1.5 text-[12px] text-slate-400 transition hover:bg-slate-700/40 disabled:opacity-60"
            >
              稍后
            </button>
          </div>
        </div>

        <!-- 更新过程 / 结果 -->
        <p v-if="updating" class="text-[12px] text-amber-300">{{ updateMsg }}</p>
        <p v-if="updateError" class="text-[12px] text-red-400">{{ updateError }}</p>
        <p v-if="updateInfo.error && !updateInfo.has_update" class="text-[11px] text-slate-600">
          版本检查失败：{{ updateInfo.error }}
        </p>
      </div>

      <!-- 失败态 -->
      <div v-if="isFailed" class="mt-6 animate-fade-up">
        <div class="flex items-start gap-3 rounded-xl border border-red-500/30 bg-red-500/10 p-4">
          <TriangleAlert class="mt-0.5 shrink-0" :size="18" color="#EF4444" />
          <div class="min-w-0 text-[13px] leading-relaxed text-red-200/90">
            {{ errorText || "dsh 服务启动失败，请查看下方日志。" }}
          </div>
        </div>

        <div class="mt-4 flex gap-3">
          <button
            @click="onRetry"
            :disabled="retrying"
            class="flex flex-1 items-center justify-center gap-2 rounded-lg border border-cyan-400/40 bg-cyan-400/10 px-4 py-2.5 text-[13px] font-medium text-cyan-300 transition hover:bg-cyan-400/20 disabled:opacity-60"
          >
            <RotateCw :size="15" :class="{ 'animate-spin': retrying }" />
            {{ retrying ? "正在重启…" : "重试启动" }}
          </button>
          <button
            @click="openLogDir"
            class="flex flex-1 items-center justify-center gap-2 rounded-lg border border-slate-600/50 bg-slate-800/40 px-4 py-2.5 text-[13px] font-medium text-slate-300 transition hover:bg-slate-700/40"
          >
            <FolderOpen :size="15" />
            打开日志目录
          </button>
        </div>

        <button
          @click="showLog = !showLog"
          class="mt-3 w-full text-center font-mono text-[12px] text-slate-400 transition hover:text-cyan-300"
        >
          {{ showLog ? "收起日志" : "展开日志" }} ▾
        </button>

        <div
          v-if="showLog"
          class="mt-3 max-h-56 overflow-auto rounded-lg border border-slate-700/50 bg-black/40 p-3"
        >
          <pre class="whitespace-pre-wrap break-all font-mono text-[12px] leading-relaxed text-slate-300">{{ loadingLog ? "加载中…" : log || "（无日志）" }}</pre>
        </div>
      </div>

      <p class="mt-8 text-center font-mono text-[11px] tracking-wide text-slate-600">
        deepseek-harness · everything is a plugin
      </p>
    </div>
  </div>
</template>

<style scoped>
  @keyframes loadbar {
    0% {
      width: 24%;
      margin-left: 0;
    }
    50% {
      width: 55%;
      margin-left: 25%;
    }
    100% {
      width: 24%;
      margin-left: 76%;
    }
  }
</style>
