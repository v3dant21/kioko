const base = '/api/v2/albums'
const maxPhoto = 32 * 1024 * 1024

async function draftStore(action, value) {
  const db = await new Promise((resolve, reject) => {
    const request = indexedDB.open('kioku-uploads', 1)
    request.onupgradeneeded = () => request.result.createObjectStore('drafts')
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error)
  })
  try {
    return await new Promise((resolve, reject) => {
      const tx = db.transaction('drafts', action === 'get' ? 'readonly' : 'readwrite')
      const store = tx.objectStore('drafts')
      const request = action === 'put' ? store.put(value, 'current') : store[action]('current')
      tx.oncomplete = () => resolve(request.result)
      tx.onabort = () => reject(tx.error || Error('Could not save the draft'))
    })
  } finally { db.close() }
}
export const loadDraft = () => draftStore('get')
export const forgetDraft = () => draftStore('delete')

async function fingerprint(file) {
  const worker = new Worker(new URL('./hash.worker.js', import.meta.url))
  try {
    return await new Promise((resolve, reject) => {
      worker.onmessage = ({ data }) => data.error ? reject(Error(data.error)) : resolve(data.digest)
      worker.onerror = () => reject(Error('Could not fingerprint photo'))
      worker.postMessage(file)
    })
  } finally { worker.terminate() }
}

export async function prepare(items, title, days, signal) {
  if (!items.length) throw Error('Choose at least one photo')
  if (items.length > 2000 || items.reduce((n, i) => n + i.file.size, 0) > 1024 ** 3)
    throw Error('Each album can hold up to 2,000 photos or 1 GiB')
  const photos = []
  for (const { id, file, tags } of items) {
    signal.throwIfAborted()
    if (!file.size || file.size > maxPhoto) throw Error(`${file.name} must be between 1 byte and 32 MiB`)
    photos.push({ id, name: file.name, size: file.size, digest: await fingerprint(file), tags })
  }
  const secret = Array.from(crypto.getRandomValues(new Uint8Array(32)), b => b.toString(16).padStart(2, '0')).join('')
  const job = { secret, manifest: { id: crypto.randomUUID(), title, days, photos } }
  await draftStore('put', job)
  return job
}

export async function request(url, options) {
  const response = await fetch(url, options)
  const body = await response.json().catch(() => ({}))
  if (!response.ok) {
    const error = Error(body.error || `Request failed (${response.status})`)
    error.status = response.status
    throw error
  }
  return body
}

export async function publish(job, items, signal, onProgress) {
  const { manifest, secret } = job
  const root = base + '/' + manifest.id
  const send = (url, method, body, headers = {}) => request(url, {
    signal, method, body, headers: { Authorization: 'Bearer ' + secret, ...headers }
  })
  const draft = await send(base, 'POST', JSON.stringify(manifest), { 'Content-Type': 'application/json' })
  const total = manifest.photos.reduce((n, photo) => n + photo.size, 0)
  let current = 0
  for (const photo of draft.published ? [] : manifest.photos) {
    const session = root + '/uploads/' + photo.id
    const { accepted, ready, chunk_size: chunk } = await send(session, 'GET')
    const confirmed = new Set(accepted)
    const partSize = n => Math.min(chunk, photo.size - n * chunk)
    current += ready ? photo.size : accepted.reduce((sum, n) => sum + partSize(n), 0)
    const report = () => onProgress({ current, total, name: photo.name })
    report()
    if (ready) continue
    const pending = Array.from({ length: Math.ceil(photo.size / chunk) }, (_, i) => i).filter(i => !confirmed.has(i))
    if (pending.length) {
      let file = items.find(x => x.id === photo.id)?.file
      if (!file) {
        for (const item of items.filter(x => x.file.name === photo.name && x.file.size === photo.size)) {
          if (await fingerprint(item.file) === photo.digest) { file = item.file; break }
        }
      }
      if (!file) throw Error(`Add the original file “${photo.name}” to the workspace, then resume.`)
      // Settle each group before returning an error so a retry cannot race active requests.
      for (let i = 0; i < pending.length; i += 3) {
        const results = await Promise.allSettled(pending.slice(i, i + 3).map(async n => {
          const blob = file.slice(n * chunk, n * chunk + partSize(n))
          const digest = await fingerprint(blob)
          await send(session + '/parts/' + n, 'PUT', blob, { 'x-part-sha256': digest })
          current += blob.size
          report()
        }))
        const failed = results.find(x => x.status === 'rejected')
        if (failed) throw failed.reason
      }
    }
    await send(session + '/complete', 'POST')
  }
  return send(root + '/publish', 'POST')
}
