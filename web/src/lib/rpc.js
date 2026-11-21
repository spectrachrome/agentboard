// WebSocket client for the agentboard daemon. One connection, requests matched by id.
// The daemon answers in order, so a slow call delays later ones; the UI only polls.
export class Rpc {
  constructor(url, token, onStatus) {
    this.url = url
    this.token = token
    this.onStatus = onStatus || (() => {})
    this.pending = new Map()
    this.nextId = 1
    this.ws = null
    this.opening = null
    this.closed = false
  }

  open() {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) return Promise.resolve()
    if (this.opening) return this.opening
    this.opening = new Promise((resolve, reject) => {
      const ws = new WebSocket(this.url)
      ws.onopen = () => { this.ws = ws; this.onStatus('online'); resolve() }
      ws.onmessage = (ev) => {
        const r = JSON.parse(ev.data)
        const p = this.pending.get(r.id)
        if (!p) return
        this.pending.delete(r.id)
        r.ok ? p.resolve(r.data) : p.reject(new Error(r.error))
      }
      ws.onclose = () => {
        this.ws = null
        this.onStatus('offline')
        for (const p of this.pending.values()) p.reject(new Error('connection lost'))
        this.pending.clear()
        reject(new Error('connection failed'))
      }
      ws.onerror = () => {}
    }).finally(() => { this.opening = null })
    return this.opening
  }

  async call(op, args = {}) {
    if (this.closed) throw new Error('closed')
    await this.open()
    const id = this.nextId++
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject })
      const req = { id, op, ...args }
      if (this.token) req.token = this.token
      this.ws.send(JSON.stringify(req))
    })
  }

  close() {
    this.closed = true
    this.ws?.close()
  }
}
