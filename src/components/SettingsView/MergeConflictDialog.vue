<script setup lang="ts">
import { computed, ref } from "vue";
import type { BackupConflict, BackupConflictResolutionInput } from "../../stores/backup";

const props = defineProps<{ conflicts: BackupConflict[] }>();
const emit = defineEmits<{ cancel: []; apply: [resolutions: BackupConflictResolutionInput[]] }>();

const index = ref(0);
const showDetails = ref<Record<string, "local" | "backup" | null>>({});
const choices = ref<Record<string, "local" | "backup">>({});
const justCopied = ref<string | null>(null);

const activeConflict = computed(() => props.conflicts[index.value]);
const resolvedCount = computed(() => Object.keys(choices.value).length);
const total = computed(() => props.conflicts.length);
const canApply = computed(() => total.value > 0 && resolvedCount.value === total.value);
const remaining = computed(() => total.value - resolvedCount.value);

function conflictKey(conflict: BackupConflict) {
  return `${conflict.entity_type}:${conflict.id}`;
}

function kindLabel(entityType: string): string {
  return entityType === "quest_series" ? "Quest series" : "Quest";
}

function choose(resolution: "local" | "backup") {
  const conflict = activeConflict.value;
  if (!conflict) return;
  const key = conflictKey(conflict);
  choices.value = { ...choices.value, [key]: resolution };
  // auto-advance to next unresolved after a short delay
  setTimeout(() => {
    const ni = nextUnresolved(index.value, props.conflicts, choices.value);
    if (ni !== -1) index.value = ni;
  }, 220);
}

function nextUnresolved(cur: number, conflicts: BackupConflict[], resolutions: Record<string, "local" | "backup">): number {
  for (let i = cur + 1; i < conflicts.length; i++) {
    if (!resolutions[conflictKey(conflicts[i])]) return i;
  }
  for (let i = 0; i < cur; i++) {
    if (!resolutions[conflictKey(conflicts[i])]) return i;
  }
  return -1;
}

function selectedFor(resolution: "local" | "backup"): boolean {
  const conflict = activeConflict.value;
  return conflict ? choices.value[conflictKey(conflict)] === resolution : false;
}

function getDetailsKey(side: "local" | "backup"): string {
  return `${conflictKey(activeConflict.value)}:${side}`;
}

function isShowingDetails(side: "local" | "backup"): boolean {
  return showDetails.value[getDetailsKey(side)] != null;
}

function toggleDetails(side: "local" | "backup") {
  const key = getDetailsKey(side);
  showDetails.value = { ...showDetails.value, [key]: showDetails.value[key] ? null : side };
}

function metaEntries(data: unknown): Array<{ label: string; value: string }> {
  if (!data || typeof data !== "object" || Array.isArray(data)) return [];
  return Object.entries(data as Record<string, unknown>)
    .filter(([k]) => !["id", "space_id"].includes(k))
    .slice(0, 6)
    .map(([k, v]) => ({ label: k.replace(/_/g, " "), value: String(v ?? "") }));
}

function isDiff(label: string): boolean {
  const c = activeConflict.value;
  if (!c) return false;
  const key = label.replace(/ /g, "_");
  const localVal = (c.local as Record<string, unknown>)?.[key];
  const backupVal = (c.backup as Record<string, unknown>)?.[key];
  return String(localVal ?? "") !== String(backupVal ?? "");
}

function copyColumn(side: "local" | "backup") {
  const c = activeConflict.value;
  if (!c) return;
  const roleLabel = side === "local" ? "Local" : "Backup";
  const summary = side === "local" ? c.local_summary : c.backup_summary;
  const entries = metaEntries(side === "local" ? c.local : c.backup);
  const metaText = entries.map(({ label, value }) => `${label}: ${value}`).join("\n");
  const text = `[${roleLabel}] ${c.title}\n${summary}${metaText ? "\n" + metaText : ""}`;
  void navigator.clipboard?.writeText?.(text).catch(() => undefined);
  justCopied.value = side;
  setTimeout(() => (justCopied.value = null), 1200);
}

function applyResolutions() {
  if (!canApply.value) return;
  emit("apply", props.conflicts.map((c) => ({
    entity_type: c.entity_type,
    id: c.id,
    resolution: choices.value[conflictKey(c)],
  })));
}
</script>

<template>
  <Teleport to="body">
    <div class="fixed inset-0 z-[1250] flex items-end justify-center p-3 sm:items-center sm:p-4" data-testid="merge-conflict-dialog">
      <button type="button" class="absolute inset-0 bg-black/45" aria-label="Close conflict dialog" @click="emit('cancel')"></button>
      <div class="relative w-full max-w-3xl rounded-xl bg-base-100 shadow-2xl">

        <!-- Header -->
        <div class="flex items-start gap-3 p-4 pb-3">
          <div class="flex-1">
            <div class="mb-1 flex items-center gap-2">
              <span
                class="badge badge-sm"
                :class="activeConflict?.entity_type === 'quest_series' ? 'badge-secondary' : 'badge-primary'"
              >{{ activeConflict ? kindLabel(activeConflict.entity_type) : "" }}</span>
            </div>
            <h3 class="text-base font-semibold">{{ activeConflict?.title ?? "Resolve conflicts" }}</h3>
          </div>
          <span
            class="rounded-full bg-base-200 px-2 py-1 text-xs font-semibold"
            :aria-label="`${resolvedCount} of ${total} resolved`"
            data-testid="merge-conflict-counter"
          ><strong>{{ resolvedCount }}</strong>/{{ total }}</span>
        </div>

        <!-- Columns -->
        <div v-if="activeConflict" class="grid grid-cols-1 gap-3 border-t border-base-200 p-4 sm:grid-cols-2">
          <!-- Local column -->
          <section
            class="flex min-h-52 flex-col rounded-lg border border-base-300"
            :class="selectedFor('local') ? 'border-primary ring-1 ring-primary' : ''"
          >
            <div class="flex items-center justify-between border-b border-base-200 px-3 py-2">
              <span class="text-sm font-semibold">Local</span>
              <span
                v-if="selectedFor('local')"
                class="rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary"
              >Keeping local</span>
            </div>
            <div class="flex-1 px-3 py-2">
              <p class="text-sm opacity-80">{{ activeConflict.local_summary }}</p>
              <dl v-if="metaEntries(activeConflict.local).length" class="mt-2 grid grid-cols-[auto,1fr] gap-x-3 gap-y-0.5 text-xs">
                <template v-for="entry in metaEntries(activeConflict.local)" :key="entry.label">
                  <dt class="opacity-50">{{ entry.label }}</dt>
                  <dd :class="isDiff(entry.label) ? 'font-medium text-warning' : ''">{{ entry.value }}</dd>
                </template>
              </dl>
              <pre v-if="isShowingDetails('local')" class="mt-2 max-h-32 overflow-auto rounded bg-base-200 p-2 text-xs">{{ JSON.stringify(activeConflict.local, null, 2) }}</pre>
            </div>
            <div class="flex items-center gap-2 border-t border-base-200 px-3 py-2">
              <button class="btn btn-ghost btn-xs" @click="copyColumn('local')">{{ justCopied === 'local' ? 'Copied' : 'Copy' }}</button>
              <button class="btn btn-ghost btn-xs" @click="toggleDetails('local')">{{ isShowingDetails('local') ? 'Hide' : 'Show' }}</button>
            </div>
            <div class="border-t border-base-200 px-3 py-2">
              <button
                class="btn btn-sm w-full"
                :class="selectedFor('local') ? 'btn-primary' : 'btn-outline'"
                @click="choose('local')"
              >Use local</button>
            </div>
          </section>

          <!-- Backup column -->
          <section
            class="flex min-h-52 flex-col rounded-lg border border-base-300"
            :class="selectedFor('backup') ? 'border-primary ring-1 ring-primary' : ''"
          >
            <div class="flex items-center justify-between border-b border-base-200 px-3 py-2">
              <span class="text-sm font-semibold">Backup</span>
              <span
                v-if="selectedFor('backup')"
                class="rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary"
              >Using backup</span>
            </div>
            <div class="flex-1 px-3 py-2">
              <p class="text-sm opacity-80">{{ activeConflict.backup_summary }}</p>
              <dl v-if="metaEntries(activeConflict.backup).length" class="mt-2 grid grid-cols-[auto,1fr] gap-x-3 gap-y-0.5 text-xs">
                <template v-for="entry in metaEntries(activeConflict.backup)" :key="entry.label">
                  <dt class="opacity-50">{{ entry.label }}</dt>
                  <dd :class="isDiff(entry.label) ? 'font-medium text-warning' : ''">{{ entry.value }}</dd>
                </template>
              </dl>
              <pre v-if="isShowingDetails('backup')" class="mt-2 max-h-32 overflow-auto rounded bg-base-200 p-2 text-xs">{{ JSON.stringify(activeConflict.backup, null, 2) }}</pre>
            </div>
            <div class="flex items-center gap-2 border-t border-base-200 px-3 py-2">
              <button class="btn btn-ghost btn-xs" @click="copyColumn('backup')">{{ justCopied === 'backup' ? 'Copied' : 'Copy' }}</button>
              <button class="btn btn-ghost btn-xs" @click="toggleDetails('backup')">{{ isShowingDetails('backup') ? 'Hide' : 'Show' }}</button>
            </div>
            <div class="border-t border-base-200 px-3 py-2">
              <button
                class="btn btn-sm w-full"
                :class="selectedFor('backup') ? 'btn-primary' : 'btn-outline'"
                @click="choose('backup')"
              >Use backup</button>
            </div>
          </section>
        </div>

        <!-- Footer with wizard pager -->
        <div class="flex flex-wrap items-center gap-2 border-t border-base-200 px-4 py-3">
          <div class="flex items-center gap-1" aria-label="Conflict pager">
            <button
              class="btn btn-ghost btn-sm px-2"
              :disabled="index === 0"
              aria-label="Previous conflict"
              @click="index = Math.max(0, index - 1)"
            >‹</button>
            <span class="text-xs opacity-60">{{ index + 1 }} of {{ total }}</span>
            <button
              class="btn btn-ghost btn-sm px-2"
              :disabled="index >= total - 1"
              aria-label="Next conflict"
              @click="index = Math.min(total - 1, index + 1)"
            >›</button>
          </div>
          <span class="flex-1"></span>
          <span v-if="!canApply" class="text-xs opacity-40">{{ remaining }} remaining</span>
          <button class="btn btn-ghost btn-sm" @click="emit('cancel')">Cancel</button>
          <button
            class="btn btn-primary btn-sm"
            :disabled="!canApply"
            @click="applyResolutions"
          >Apply{{ canApply ? ` ${total} resolution${total === 1 ? '' : 's'}` : "" }}</button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
