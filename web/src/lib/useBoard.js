import { reactive, computed } from 'vue'
import { Rpc } from './rpc.js'

const LS = 'agentboard.session'
const POLL_MS = 2000

const state = reactive({
  token: '',
  me: null,
  meType: 'agent',
  status: 'offline',
  error: '',
  channels: [],
  agents: [],
  queues: [],
  active: null, // { kind: 'channel' | 'queue', name }
  messages: {}, // channel -> { list, lastId }
  seen: {}, // channel -> last_id the user has looked at
  tasks: [],
})

let rpc = null
let timer = null
let busy = false

// The UI is served by the daemon's human listener, so it talks to the same origin
// (in development, Vite proxies /ws to the daemon).
const wsUrl = () => `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`

// The human token is kept in sessionStorage only: it dies with the tab and is not
// written to the browser profile where other processes on this machine could read it.
export function loadToken() {
  try {
    return sessionStorage.getItem(LS) || ''
  } catch {
    return ''
  }
}

export async function connect(token) {
  disconnect()
  state.token = token
  rpc = new Rpc(wsUrl(), token, (s) => (state.status = s))
  const who = await rpc.call('whoami') // throws on a bad token / unreachable daemon
  if (who.type !== 'human') throw new Error('this token does not belong to a human')
  state.me = who.name
  state.meType = who.type
  try { sessionStorage.setItem(LS, token) } catch {}
  await tick()
  timer = setInterval(tick, POLL_MS)
}

export function disconnect() {
  clearInterval(timer)
  timer = null
  rpc?.close()
  rpc = null
  Object.assign(state, {
    me: null, meType: 'agent', status: 'offline', channels: [], agents: [], queues: [],
    active: null, messages: {}, seen: {}, tasks: [],
  })
}

export function logout() {
  try { sessionStorage.removeItem(LS) } catch {}
  disconnect()
}

async function tick() {
  if (busy || !rpc) return
  busy = true
  try {
    const [c, a, q] = await Promise.all([rpc.call('channels'), rpc.call('list_agents'), rpc.call('list_queues')])
    state.channels = c.channels
    state.agents = a.agents
    state.queues = q.queues
    state.error = ''
    if (!state.active && state.channels.length) state.active = { kind: 'channel', name: state.channels[0].name }
    const act = state.active
    if (act?.kind === 'channel') await pullMessages(act.name)
    if (act?.kind === 'queue') state.tasks = (await rpc.call('list_tasks', { queue: act.name, limit: 200 })).tasks
  } catch (e) {
    state.error = e.message
  } finally {
    busy = false
  }
}

async function pullMessages(channel) {
  const slot = (state.messages[channel] ??= { list: [], lastId: 0 })
  // Humans get the supervisor view (all messages, incl. other agents' directed ones).
  const r = await rpc.call('observe', { channel, since: slot.lastId, limit: 200 })
  if (r.messages.length) {
    slot.list.push(...r.messages)
    slot.lastId = r.messages[r.messages.length - 1].id
  }
  const ch = state.channels.find((c) => c.name === channel)
  if (ch) state.seen[channel] = ch.last_id
}

export function select(kind, name) {
  state.active = { kind, name }
  tick()
}

export const activeChannel = computed(() =>
  state.active?.kind === 'channel' ? state.channels.find((c) => c.name === state.active.name) : null)

export const activeMessages = computed(() =>
  state.active?.kind === 'channel' ? state.messages[state.active.name]?.list ?? [] : [])

export function hasUnread(channel) {
  return channel.last_id > (state.seen[channel.name] ?? 0)
}

export async function send(body, to) {
  await rpc.call('post', { channel: state.active.name, body, ...(to ? { to } : {}) })
  await tick()
}

export async function createChannel(name, description) {
  await rpc.call('create_channel', { name, description })
  state.active = { kind: 'channel', name }
  await tick()
}

export async function enqueueTask(queue, payload) {
  await rpc.call('enqueue_task', { queue, payload })
  state.active = { kind: 'queue', name: queue }
  await tick()
}

export function useBoard() {
  return state
}

// Stable color per name for avatars.
export function hue(name) {
  let h = 0
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) % 360
  return h
}
