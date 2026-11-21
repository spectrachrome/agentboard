<script setup>
import { computed, ref } from 'vue'
import { useBoard, enqueueTask } from '../lib/useBoard.js'

const board = useBoard()
const payload = ref('')
const error = ref('')

const columns = computed(() => [
  { key: 'pending', title: 'Pending', dot: 'bg-amber-400' },
  { key: 'claimed', title: 'In progress', dot: 'bg-sky-400' },
  { key: 'done', title: 'Done', dot: 'bg-emerald-400' },
].map((c) => ({ ...c, tasks: board.tasks.filter((t) => t.status === c.key) })))

async function add() {
  error.value = ''
  try {
    await enqueueTask(board.active.name, payload.value.trim())
    payload.value = ''
  } catch (e) { error.value = e.message }
}

const leaseLeft = (t) => {
  const s = t.lease_until - Math.floor(Date.now() / 1000)
  return s > 0 ? `${Math.ceil(s / 60)}m left` : 'lease expired'
}
</script>

<template>
  <header class="flex items-center gap-4 border-b border-white/10 px-6 py-3">
    <h1 class="text-base font-semibold text-white">Queue · {{ board.active.name }}</h1>
    <form class="ml-auto flex w-full max-w-md gap-2" @submit.prevent="add">
      <input v-model="payload" placeholder="New task…" class="min-w-0 flex-1 rounded-md bg-white/5 px-3 py-1.5 text-sm text-white outline-1 -outline-offset-1 outline-white/10 placeholder:text-slate-500 focus:outline-2 focus:-outline-offset-2 focus:outline-indigo-500" />
      <button :disabled="!payload.trim()" class="rounded-md bg-indigo-500 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-400 disabled:opacity-40">Add</button>
    </form>
  </header>
  <p v-if="error" class="px-6 pt-2 text-xs text-red-400">{{ error }}</p>

  <div class="grid flex-1 grid-cols-1 gap-4 overflow-y-auto p-6 md:grid-cols-3">
    <section v-for="c in columns" :key="c.key" class="rounded-xl bg-white/[0.03] p-3 outline-1 outline-white/10">
      <h2 class="flex items-center gap-2 px-1 pb-3 text-sm font-semibold text-white">
        <span class="size-2 rounded-full" :class="c.dot"></span>{{ c.title }}
        <span class="ml-auto text-xs font-normal text-slate-500">{{ c.tasks.length }}</span>
      </h2>
      <ul class="space-y-2">
        <li v-for="t in c.tasks" :key="t.id" class="rounded-lg bg-slate-900 p-3 text-sm outline-1 outline-white/10">
          <p class="whitespace-pre-wrap break-words text-slate-200">{{ t.payload }}</p>
          <p v-if="t.result" class="mt-2 whitespace-pre-wrap break-words rounded bg-emerald-500/10 px-2 py-1 text-emerald-200">{{ t.result }}</p>
          <p class="mt-2 flex flex-wrap gap-x-3 text-xs text-slate-500">
            <span>#{{ t.id }}</span>
            <span>by {{ t.created_by }}</span>
            <span v-if="t.claimed_by">→ {{ t.claimed_by }}</span>
            <span v-if="t.status === 'claimed'">{{ leaseLeft(t) }}</span>
          </p>
        </li>
        <li v-if="!c.tasks.length" class="px-1 text-xs text-slate-600">Empty</li>
      </ul>
    </section>
  </div>
</template>
