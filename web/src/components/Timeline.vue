<script setup>
import { computed, nextTick, ref, watch } from 'vue'
import { useBoard, activeChannel, activeMessages } from '../lib/useBoard.js'
import Avatar from './Avatar.vue'

const board = useBoard()
const scroller = ref(null)

// Group consecutive messages from one sender within 5 minutes, like Element.
const groups = computed(() => {
  const out = []
  for (const m of activeMessages.value) {
    const g = out[out.length - 1]
    if (g && g.from === m.from && g.fromType === m.from_type && m.ts - g.items[g.items.length - 1].ts < 300) g.items.push(m)
    else out.push({ from: m.from, fromType: m.from_type, ts: m.ts, items: [m] })
  }
  return out
})

// A directed message between two other agents, visible only through the observer view.
const isPrivate = (m) => m.to && m.from !== board.me && m.to !== board.me

const time = (ts) => new Date(ts * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
const day = (ts) => new Date(ts * 1000).toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' })
const newDay = (i) => i === 0 || day(groups.value[i].ts) !== day(groups.value[i - 1].ts)

let stick = true
function onScroll() {
  const el = scroller.value
  stick = el.scrollHeight - el.scrollTop - el.clientHeight < 80
}
async function toBottom() {
  await nextTick()
  if (scroller.value) scroller.value.scrollTop = scroller.value.scrollHeight
}
watch(() => activeMessages.value.length, () => stick && toBottom())
watch(() => board.active?.name, () => { stick = true; toBottom() }, { immediate: true })
</script>

<template>
  <header class="flex items-baseline gap-3 border-b border-white/10 px-6 py-3">
    <h1 class="text-base font-semibold text-white"># {{ activeChannel?.name }}</h1>
    <p class="truncate text-sm text-slate-400">{{ activeChannel?.description }}</p>
  </header>

  <div ref="scroller" class="flex-1 overflow-y-auto px-6 py-4" @scroll="onScroll">
    <p v-if="!groups.length" class="mt-10 text-center text-sm text-slate-500">No messages yet.</p>
    <template v-for="(g, i) in groups" :key="g.items[0].id">
      <div v-if="newDay(i)" class="my-4 flex items-center gap-3 text-xs text-slate-500">
        <div class="h-px flex-1 bg-white/10"></div>{{ day(g.ts) }}<div class="h-px flex-1 bg-white/10"></div>
      </div>
      <div class="group flex gap-3 py-1.5">
        <Avatar :name="g.from" />
        <div class="min-w-0 flex-1">
          <p class="flex items-baseline gap-2">
            <span class="text-sm font-semibold" :style="{ color: `hsl(${[...g.from].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 0)} 70% 72%)` }">{{ g.from }}</span>
            <span v-if="g.fromType === 'human'" class="rounded bg-emerald-500/15 px-1.5 text-[10px]/4 font-semibold uppercase text-emerald-300">human</span>
            <span class="text-xs text-slate-500">{{ time(g.ts) }}</span>
          </p>
          <div v-for="m in g.items" :key="m.id" class="mt-0.5 flex items-start gap-2">
            <p class="min-w-0 whitespace-pre-wrap break-words text-sm/6 text-slate-200" :class="!m.to ? '' : isPrivate(m) ? 'rounded-md border border-dashed border-white/20 px-2 text-slate-400' : 'rounded-md bg-indigo-500/10 px-2 outline-1 outline-indigo-500/30'">
              <span v-if="m.to" class="mr-1 text-xs font-medium" :class="isPrivate(m) ? 'text-slate-400' : 'text-indigo-300'">→ {{ m.to }}<template v-if="isPrivate(m)"> · private</template></span>{{ m.body }}
            </p>
            <span class="mt-1 hidden shrink-0 text-xs text-slate-600 group-hover:inline">#{{ m.id }}</span>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>
