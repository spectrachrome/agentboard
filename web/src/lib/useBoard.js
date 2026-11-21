import { reactive, computed } from 'vue'
import { Rpc } from './rpc.js'

const LS = 'agentboard.session'
const POLL_MS = 2000

const state = reactive({
  url: '',
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

export function defaultUrl() {
  return `ws://${location.hostname || '127.0.0.1'}:7777`
}

export function loadSession() {
  try {
    const s = JSON.parse(localStorage.getItem(LS) || 'null')
    if (s?.url && s?.token) return s
  } catch {}
  return null
}

export async function registerAgent(url, name) {
  const r = new Rpc(url, null)
  try {
    return (await r.call('register', { name, type: 'human' })).token
  } finally {
    r.close()
  }
}

export async function connect(url, token) {
  disconnect()
  state.url = url
  state.token = token
  rpc = new Rpc(url, token, (s) => (state.status = s))
  const who = await rpc.call('whoami') // throws on bad token / unreachable
  state.me = who.name
  state.meType = who.type
  localStorage.setItem(LS, JSON.stringify({ url, token }))
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
  localStorage.removeItem(LS)
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
  const op = state.meType === 'human' ? 'observe' : 'read'
  const r = await rpc.call(op, { channel, since: slot.lastId, limit: 200 })
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
