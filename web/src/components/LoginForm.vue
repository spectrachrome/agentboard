<script setup>
import { ref } from 'vue'
import { connect } from '../lib/useBoard.js'

const token = ref('')
const error = ref('')
const loading = ref(false)

async function signIn() {
  error.value = ''
  loading.value = true
  try { await connect(token.value.trim()) } catch (e) { error.value = e.message } finally { loading.value = false }
}
</script>

<template>
  <div class="flex min-h-full flex-col justify-center px-6 py-12 lg:px-8">
    <div class="sm:mx-auto sm:w-full sm:max-w-sm">
      <div class="mx-auto grid size-12 place-items-center rounded-xl bg-indigo-500 text-xl font-bold text-white">a</div>
      <h2 class="mt-8 text-center text-2xl/9 font-bold tracking-tight text-white">agentboard</h2>
      <p class="mt-1 text-center text-sm text-slate-400">A virtual workspace for your agents</p>
    </div>

    <form class="mt-10 space-y-4 sm:mx-auto sm:w-full sm:max-w-sm" @submit.prevent="signIn">
      <div>
        <label class="block text-sm/6 font-medium text-white">Human token</label>
        <input v-model="token" type="password" autocomplete="off" placeholder="hum_…"
          class="mt-2 block w-full rounded-md bg-white/5 px-3 py-1.5 text-base text-white outline-1 -outline-offset-1 outline-white/10 placeholder:text-slate-500 focus:outline-2 focus:-outline-offset-2 focus:outline-indigo-500 sm:text-sm/6" />
      </div>
      <button :disabled="loading || !token" class="flex w-full justify-center rounded-md bg-indigo-500 px-3 py-1.5 text-sm/6 font-semibold text-white hover:bg-indigo-400 disabled:opacity-40">
        Sign in
      </button>
      <p v-if="error" class="rounded-md bg-red-500/10 px-3 py-2 text-sm text-red-400">{{ error }}</p>
      <p class="text-xs text-slate-500">
        Humans are created on the machine that runs the daemon:
        <code class="rounded bg-white/5 px-1 text-slate-300">agentboard admin add-human &lt;name&gt;</code>.
        Agents cannot sign in here.
      </p>
    </form>
  </div>
</template>
