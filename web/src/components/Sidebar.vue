<script setup>
import { ref } from 'vue'
import { useBoard, select, hasUnread, createChannel, logout } from '../lib/useBoard.js'
import Avatar from './Avatar.vue'

const board = useBoard()
const adding = ref(false)
const newName = ref('')
const newDesc = ref('')
const err = ref('')

async function add() {
  err.value = ''
  try {
    await createChannel(newName.value.trim(), newDesc.value.trim())
    adding.value = false
    newName.value = newDesc.value = ''
  } catch (e) { err.value = e.message }
}

function newQueue() {
  const n = window.prompt('Queue name (a-z, 0-9, - _)')?.trim()
  if (n) select('queue', n)
}

const online = (a) => Date.now() / 1000 - a.last_seen < 120
const isActive = (kind, name) => board.active?.kind === kind && board.active.name === name
</script>

<template>
  <aside class="flex w-64 shrink-0 flex-col border-r border-white/10 bg-slate-900">
    <div class="flex items-center gap-3 border-b border-white/10 px-4 py-3">
      <Avatar :name="board.me" />
      <div class="min-w-0 flex-1">
        <p class="truncate text-sm font-semibold text-white">{{ board.me }}</p>
        <p class="flex items-center gap-1.5 text-xs text-slate-400">
          <span class="size-1.5 rounded-full" :class="board.status === 'online' && !board.error ? 'bg-emerald-400' : 'bg-red-400'"></span>
          {{ board.error || board.status }}
        </p>
      </div>
      <button class="text-xs text-slate-500 hover:text-slate-300" @click="logout">Sign out</button>
    </div>

    <nav class="flex-1 overflow-y-auto px-2 py-4">
      <div class="flex items-center justify-between px-2">
        <h3 class="text-xs/6 font-semibold uppercase tracking-wide text-slate-500">Channels</h3>
        <button class="rounded p-0.5 text-slate-400 hover:bg-white/10 hover:text-white" title="New channel" @click="adding = !adding">
          <svg class="size-4" viewBox="0 0 20 20" fill="currentColor"><path d="M10.75 4.75a.75.75 0 0 0-1.5 0v4.5h-4.5a.75.75 0 0 0 0 1.5h4.5v4.5a.75.75 0 0 0 1.5 0v-4.5h4.5a.75.75 0 0 0 0-1.5h-4.5z"/></svg>
        </button>
      </div>

      <form v-if="adding" class="mt-2 space-y-2 rounded-md bg-white/5 p-2" @submit.prevent="add">
        <input v-model="newName" placeholder="channel-name" class="block w-full rounded bg-slate-950 px-2 py-1 text-sm text-white outline-1 outline-white/10 focus:outline-indigo-500" />
        <input v-model="newDesc" placeholder="description (optional)" class="block w-full rounded bg-slate-950 px-2 py-1 text-sm text-white outline-1 outline-white/10 focus:outline-indigo-500" />
        <p v-if="err" class="text-xs text-red-400">{{ err }}</p>
        <button class="w-full rounded bg-indigo-500 py-1 text-sm font-semibold text-white hover:bg-indigo-400">Create</button>
      </form>

      <ul class="mt-1 space-y-0.5">
        <li v-for="c in board.channels" :key="c.name">
          <button class="group flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm/6"
            :class="isActive('channel', c.name) ? 'bg-white/10 text-white' : 'text-slate-400 hover:bg-white/5 hover:text-white'"
            @click="select('channel', c.name)">
            <span class="text-slate-500">#</span>
            <span class="min-w-0 flex-1 truncate" :class="hasUnread(c) && !isActive('channel', c.name) ? 'font-semibold text-white' : ''">{{ c.name }}</span>
            <span v-if="hasUnread(c) && !isActive('channel', c.name)" class="size-2 rounded-full bg-indigo-400"></span>
          </button>
        </li>
      </ul>

      <h3 class="mt-6 px-2 text-xs/6 font-semibold uppercase tracking-wide text-slate-500">Task queues</h3>
      <ul class="mt-1 space-y-0.5">
        <li v-for="q in board.queues" :key="q.queue">
          <button class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm/6"
            :class="isActive('queue', q.queue) ? 'bg-white/10 text-white' : 'text-slate-400 hover:bg-white/5 hover:text-white'"
            @click="select('queue', q.queue)">
            <span class="min-w-0 flex-1 truncate">{{ q.queue }}</span>
            <span v-if="q.pending" class="rounded-full bg-amber-400/15 px-2 text-xs/5 font-medium text-amber-300">{{ q.pending }}</span>
          </button>
        </li>
        <li v-if="!board.queues.length" class="px-2 text-xs text-slate-600">No queues yet</li>
        <li>
          <button class="px-2 py-1 text-xs text-slate-500 hover:text-slate-300" @click="newQueue">+ New queue</button>
        </li>
      </ul>

      <h3 class="mt-6 px-2 text-xs/6 font-semibold uppercase tracking-wide text-slate-500">Agents</h3>
      <ul class="mt-1 space-y-1">
        <li v-for="a in board.agents" :key="a.name" class="flex items-center gap-2 px-2 py-1 text-sm text-slate-300">
          <span class="relative">
            <Avatar :name="a.name" size="size-6" />
            <span class="absolute -right-0.5 -bottom-0.5 size-2 rounded-full ring-2 ring-slate-900" :class="online(a) ? 'bg-emerald-400' : 'bg-slate-600'"></span>
          </span>
          <span class="truncate">{{ a.name }}</span>
          <span v-if="a.type === 'human'" class="ml-auto rounded bg-white/10 px-1.5 text-[10px]/4 font-medium uppercase text-slate-400">human</span>
        </li>
      </ul>
    </nav>
  </aside>
</template>
