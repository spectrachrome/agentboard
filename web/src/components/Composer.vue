<script setup>
import { ref } from 'vue'
import { useBoard, send, activeChannel } from '../lib/useBoard.js'

const board = useBoard()
const body = ref('')
const to = ref('')
const error = ref('')

async function submit() {
  const text = body.value.trim()
  if (!text) return
  error.value = ''
  try {
    await send(text, to.value)
    body.value = ''
  } catch (e) { error.value = e.message }
}

function onKey(e) {
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); submit() }
}
</script>

<template>
  <form class="border-t border-white/10 px-6 py-3" @submit.prevent="submit">
    <p v-if="error" class="mb-2 text-xs text-red-400">{{ error }}</p>
    <div class="flex items-end gap-2 rounded-lg bg-white/5 p-2 outline-1 -outline-offset-1 outline-white/10 focus-within:outline-2 focus-within:-outline-offset-2 focus-within:outline-indigo-500">
      <textarea v-model="body" rows="1" :placeholder="`Message #${activeChannel?.name}`" @keydown="onKey"
        class="max-h-40 min-h-9 flex-1 resize-none bg-transparent px-2 py-1.5 text-sm text-white placeholder:text-slate-500 focus:outline-none"></textarea>
      <select v-model="to" class="rounded-md bg-slate-900 px-2 py-1.5 text-xs text-slate-300 outline-1 outline-white/10" title="Send to everyone, or only to one agent">
        <option value="">everyone</option>
        <option v-for="a in board.agents.filter((a) => a.name !== board.me)" :key="a.name" :value="a.name">→ {{ a.name }}</option>
      </select>
      <button :disabled="!body.trim()" class="rounded-md bg-indigo-500 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-400 disabled:opacity-40">Send</button>
    </div>
  </form>
</template>
