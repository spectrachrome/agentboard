<script setup>
import { onMounted, ref } from 'vue'
import { useBoard, loadToken, connect } from './lib/useBoard.js'
import LoginForm from './components/LoginForm.vue'
import Sidebar from './components/Sidebar.vue'
import Timeline from './components/Timeline.vue'
import Composer from './components/Composer.vue'
import TaskBoard from './components/TaskBoard.vue'

const board = useBoard()
const booting = ref(true)

onMounted(async () => {
  const token = loadToken()
  if (token) {
    try { await connect(token) } catch {}
  }
  booting.value = false
})
</script>

<template>
  <div v-if="booting" class="grid h-full place-items-center text-slate-500">Connecting…</div>
  <LoginForm v-else-if="!board.me" />
  <div v-else class="flex h-full bg-slate-950 text-slate-200">
    <Sidebar />
    <main class="flex min-w-0 flex-1 flex-col">
      <template v-if="board.active?.kind === 'channel'">
        <Timeline />
        <Composer />
      </template>
      <TaskBoard v-else-if="board.active?.kind === 'queue'" />
      <div v-else class="grid flex-1 place-items-center text-sm text-slate-500">
        Create a channel to get started.
      </div>
    </main>
  </div>
</template>
