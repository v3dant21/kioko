import { useEffect, useState } from 'react'
import { Brand, DownloadLink, SearchField } from './Studio'
import { request } from './publishing'

export default function AlbumViewer({ id }) {
  const root = '/api/v2/albums/' + id
  const [metadata, setMetadata] = useState(null)
  const [result, setResult] = useState(null)
  const [query, setQuery] = useState('')
  const [history, setHistory] = useState([''])
  const [failure, setFailure] = useState(null)
  const [expired, setExpired] = useState(false)
  const [attempt, setAttempt] = useState(0)
  const cursor = history.at(-1)
  const key = JSON.stringify([id, query, cursor, attempt])
  const page = result?.key === key ? result.page : null
  const error = failure?.key === key ? failure.message : ''

  useEffect(() => {
    const controller = new AbortController()
    const handle = e => {
      if (controller.signal.aborted) return
      if (e.status === 410) setExpired(true)
      else setFailure({ key, message: e.message })
    }
    const params = new URLSearchParams({ q: query })
    if (cursor) params.set('cursor', cursor)
    Promise.all([request(root, { signal: controller.signal }), request(root + '/photos?' + params, { signal: controller.signal })])
      .then(([info, page]) => { if (!controller.signal.aborted) { setMetadata(info); setResult({ key, page }); setFailure(null) } })
      .catch(handle)
    return () => controller.abort()
  }, [root, query, cursor, key])

  useEffect(() => {
    if (!metadata) return
    const timer = setTimeout(() => setExpired(true), Math.max(0, metadata.expires_at * 1000 - Date.now()))
    return () => clearTimeout(timer)
  }, [metadata])

  return <div className="app"><header className="header"><Brand section="Shared album" /></header>
    <main className="main shared-gallery">
    <h1 className="gallery-title">{metadata?.title || 'Shared album'}</h1>
    {expired ? <p>This album has expired.</p> : <>
      <div className="viewer-tools">{metadata && <><p>{metadata.count} photos · Expires {new Date(metadata.expires_at * 1000).toLocaleString()}</p><DownloadLink href={root + '/download'} /></>}</div>
      <SearchField label="Search names or tags" value={query} onChange={value => { setQuery(value); setHistory(['']) }} />
      {error ? <p role="alert">{error} <button onClick={() => setAttempt(x => x + 1)}>Retry</button></p> : !page ? <p>Loading photos…</p> : <>
        <div className="masonry">{page.items.map(item => <div className="viewer-photo" key={item.id}>
          <img src={root + '/photos/' + item.id} alt={item.name} loading="lazy" />
        </div>)}</div>
        {!page.items.length && <p>No matching photos</p>}
        <nav className="album-pages" aria-label="Gallery pages"><button disabled={history.length === 1} onClick={() => setHistory(x => x.slice(0, -1))}>Previous</button>
          {' '}Page {history.length}{' '}
          <button disabled={!page.next_cursor} onClick={() => setHistory(x => [...x, page.next_cursor])}>Next</button></nav>
      </>}
    </>}
  </main></div>
}
