// 实弹监听：WS 订阅 + 关键事件落库时间线（node ws 客户端，60s 超时）
const events = []
const ws = new WebSocket('ws://localhost:7101/ws')
const timer = setTimeout(() => {
  console.log(JSON.stringify(events, null, 1))
  process.exit(events.length ? 0 : 2)
}, Number(process.argv[2] ?? 60_000))
ws.onmessage = (e) => {
  const m = JSON.parse(e.data)
  if (['patrol.finished', 'task.statusChanged', 'session.statusChanged', 'task.contractAlert', 'task.contractViolated'].includes(m.name)) {
    events.push({ t: new Date().toISOString().slice(11, 19), name: m.name, data: m.data })
    console.log('EVT', events[events.length - 1].t, m.name, JSON.stringify(m.data).slice(0, 120))
  }
}
ws.onopen = () => console.log('WS-OPEN')
ws.onerror = (e) => { console.error('WS-ERR'); process.exit(3) }
