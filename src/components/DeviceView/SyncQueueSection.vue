<script setup lang="ts">
import { computed, ref } from "vue";
import { ChevronDownIcon, ChevronUpIcon } from "@heroicons/vue/24/outline";
import type { SyncQueueEntry, SyncQueueSummary } from "../../stores/device";
import { spaceCss } from "../../stores/space";
import { relativeTime } from "../../utils/timestamp";

const props = defineProps<{
  summary: SyncQueueSummary | null;
  peerName: string;
  // Whether any channel is currently able to carry the queue. Decides
  // between "sending" and "nothing can move", which is the difference
  // between a queue that is draining and one that is stuck.
  anyChannelConnected: boolean;
}>();

const open = ref(false);

const pending = computed(() => props.summary?.pending_count ?? 0);
const synced = computed(() => pending.value === 0);

const lastReached = computed(() => {
  const at = props.summary?.last_acked_at;
  return at ? relativeTime(at) : null;
});

// The entries the backend actually resolved a name for. Anything else is
// internal bookkeeping (a reminder, a checklist row, focus history) that
// the person never named, so it is counted but not listed -- inventing a
// label like "Checklist activity" would fill the list with words that mean
// nothing to the reader.
const named = computed<SyncQueueEntry[]>(
  () => props.summary?.entries.filter((entry) => entry.title) ?? [],
);

// Counted against the real total, not the array length: the backend caps
// what it sends, so "+ more" has to be computed from `pending_count`.
const remaining = computed(() => Math.max(0, pending.value - named.value.length));
</script>

<template>
  <section class="rounded-xl bg-base-200 p-3" data-testid="sync-queue-section">
    <h2 class="mb-2 text-sm font-semibold uppercase tracking-wide opacity-70">Sync queue</h2>

    <div class="overflow-hidden rounded-lg bg-base-100">
      <div v-if="synced" class="flex items-center gap-2.5 px-2 py-2.5" data-testid="sync-queue-synced">
        <span class="size-2.5 shrink-0 rounded-full bg-success" />
        <span class="min-w-0 flex-1 text-sm font-medium">
          Everything synced
          <small v-if="lastReached" class="mt-0.5 block text-[11px] font-normal text-[var(--fg-3)]">
            Last change reached {{ peerName }} {{ lastReached }}
          </small>
        </span>
      </div>

      <template v-else>
        <button
          type="button"
          class="flex w-full items-center gap-2.5 px-2 py-2.5 text-left"
          data-testid="sync-queue-toggle"
          :aria-expanded="open"
          @click="open = !open"
        >
          <span
            class="size-2.5 shrink-0 rounded-full"
            :class="anyChannelConnected ? 'bg-warning' : 'bg-[var(--fg-5)]'"
          />
          <span class="min-w-0 flex-1 text-sm font-medium">
            {{ pending }} {{ pending === 1 ? "change" : "changes" }} waiting
            <small class="mt-0.5 block text-[11px] font-normal text-[var(--fg-3)]">
              {{
                anyChannelConnected
                  ? "Sending now"
                  : `Nothing can reach ${peerName} until a channel connects`
              }}
            </small>
          </span>
          <component :is="open ? ChevronUpIcon : ChevronDownIcon" class="size-3.5 shrink-0 text-[var(--fg-4)]" />
        </button>

        <div v-if="open" class="border-t border-base-200 pb-2 pt-1" data-testid="sync-queue-body">
          <div
            v-for="entry in named"
            :key="`${entry.entity_type}:${entry.entity_id}`"
            class="flex items-center gap-2.5 px-2 py-1.5 text-[12.5px]"
            data-testid="sync-queue-entry"
          >
            <span class="size-[7px] shrink-0 rounded-sm bg-current" :class="spaceCss(entry.space_id)" />
            <span class="min-w-0 flex-1 truncate">{{ entry.title }}</span>
          </div>
          <p v-if="remaining > 0" class="px-2 pb-0.5 pt-1.5 text-[11px] text-[var(--fg-3)]">
            {{ remaining }}+ more
          </p>
        </div>
      </template>
    </div>
  </section>
</template>
