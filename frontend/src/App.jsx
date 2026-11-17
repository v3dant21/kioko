import { useState, useCallback, useEffect, useMemo, useRef } from 'react'
import './App.css'
import { MotionConfig, motion } from 'motion/react'
import { Plus, UploadSimple, ShareNetwork, Trash, Check, CaretLeft, CaretRight, SelectionAll, SelectionSlash, FolderPlus, LinkSimple, X, Copy, ImageSquare, Hourglass, DownloadSimple, Images } from '@phosphor-icons/react'
import { Brand, SearchField, StudioIntro, Lifetime, WorkspaceFooter } from './Studio'
import AlbumViewer from './AlbumViewer'
import { prepare, publish, loadDraft, forgetDraft } from './publishing'

const AnimatedBar = motion.div

const uid = () => crypto.randomUUID()

function App() {
  // Check if we're on a gallery viewer page
  const galleryMatch = window.location.pathname.match(/^\/g\/(.+)$/)
  const viewGalleryId = galleryMatch ? galleryMatch[1] : null

  if (viewGalleryId?.startsWith('v2-')) return <AlbumViewer id={viewGalleryId.slice(3)} />
  if (viewGalleryId) {
    return <GalleryViewer galleryId={viewGalleryId} />
  }

  return <MotionConfig reducedMotion="user"><UploadApp /></MotionConfig>
}

function UploadApp() {
  const [gallery, setGallery] = useState([])
  const [albums, setAlbums] = useState([])
  const [search, setSearch] = useState('')
  const [modalOpen, setModalOpen] = useState(false)
  const [albumName, setAlbumName] = useState('')
  const [error, setError] = useState('')
  const [uploading, setUploading] = useState(false)
  const [progress, setProgress] = useState({ current: 0, total: 0, name: '' })
  const [shareUrl, setShareUrl] = useState(null)
  const [showShareBox, setShowShareBox] = useState(false)

  const [page, setPage] = useState(0)
  const [days, setDays] = useState(3)
  const [job, setJob] = useState(null)
  const [restored, setRestored] = useState(false)
  const [deadline, setDeadline] = useState(null)
  const picker = useRef(null)
  const active = useRef(null)
  useEffect(() => {
    let mounted = true
    loadDraft().then(draft => { if (mounted) setJob(draft) })
      .catch(e => { if (mounted) setError(e.message) })
      .finally(() => { if (mounted) setRestored(true) })
    return () => { mounted = false; active.current?.abort() }
  }, [])
  const index = useMemo(() => new Map(gallery.map((photo, pos) =>
    [photo.id, { pos, terms: [photo.name, ...photo.tags].map(t => t.toLowerCase()) }])), [gallery])
  const term = search.trim().toLowerCase()
  const filteredGallery = gallery.filter(item => !term || index.get(item.id).terms.some(t => t.includes(term)))
  const pages = Math.max(1, Math.ceil(filteredGallery.length / 24))
  const currentPage = Math.min(page, pages - 1)
  const visible = filteredGallery.slice(currentPage * 24, (currentPage + 1) * 24)

  const selectedCount = gallery.filter(x => x.selected).length

  const addFiles = useCallback((files) => {
    const newItems = [...files]
      .filter(f => f.type.startsWith('image/'))
      .map(f => ({
        id: uid(),
        file: f,
        name: f.name,
        url: URL.createObjectURL(f),
        tags: [],
        selected: false,
        shareLink: null
      }))
    setGallery(prev => [...prev, ...newItems])
  }, [])

  const handleDrop = useCallback((e) => {
    e.preventDefault()
    e.currentTarget.classList.remove('dragover')
    if (e.dataTransfer.files.length) addFiles(e.dataTransfer.files)
  }, [addFiles])

  const handleDragOver = (e) => {
    e.preventDefault()
    e.currentTarget.classList.add('dragover')
  }

  const handleDragLeave = (e) => {
    e.currentTarget.classList.remove('dragover')
  }

  const deleteImg = (id) => {
    setGallery(prev => {
      const item = prev.find(x => x.id === id)
      if (item) URL.revokeObjectURL(item.url)
      return prev.filter(x => x.id !== id)
    })
  }

  const moveImg = (idx, dir) => {
    const newIdx = idx + dir
    if (newIdx >= 0 && newIdx < gallery.length) {
      setGallery(prev => {
        const arr = [...prev];
        [arr[idx], arr[newIdx]] = [arr[newIdx], arr[idx]]
        return arr
      })
    }
  }

  const toggleSelect = (id) => {
    setGallery(prev => prev.map(x => x.id === id ? { ...x, selected: !x.selected } : x))
  }

  const selectItems = items => {
    const ids = new Set(items.map(x => x.id))
    setGallery(prev => prev.map(x => ids.has(x.id) ? { ...x, selected: true } : x))
  }
  const deselectAll = () => setGallery(prev => prev.map(x => ({ ...x, selected: false })))

  const addTag = (id, raw) => {
    const tags = raw.split(',').map(t => t.trim()).filter(Boolean)
    setGallery(prev => prev.map(x => {
      if (x.id === id) {
        const newTags = [...x.tags]
        tags.forEach(t => { if (!newTags.includes(t)) newTags.push(t) })
        return { ...x, tags: newTags }
      }
      return x
    }))
  }

  const removeTag = (id, tag) => {
    setGallery(prev => prev.map(x => x.id === id ? { ...x, tags: x.tags.filter(t => t !== tag) } : x))
  }

  const clearAll = () => {
    gallery.forEach(x => URL.revokeObjectURL(x.url))
    setGallery([])
  }

  const createAlbum = () => {
    const selected = gallery.filter(x => x.selected)
    if (!selected.length || !albumName.trim()) return
    setAlbums(prev => [...prev, { id: uid(), name: albumName.trim(), ids: selected.map(x => x.id) }])
    deselectAll()
    setModalOpen(false)
    setAlbumName('')
  }

  const deleteAlbum = (id) => {
    setAlbums(prev => prev.filter(a => a.id !== id))
  }

  const uploadToServer = async (album = null) => {
    if (active.current) return
    if (window.location.protocol === 'file:') {
      setError('Publishing requires the running Kioku server. You can organize photos and create local albums in this preview.')
      return
    }
    const controller = new AbortController()
    active.current = controller
    setError('')
    setUploading(true)
    setProgress({ current: 0, total: 0, name: 'Preparing album…' })
    try {
      let draft = job
      if (!draft) {
        const ids = album && new Set(album.ids)
        const items = gallery.filter(x => ids ? ids.has(x.id) : selectedCount ? x.selected : !x.shareLink)
        draft = await prepare(items, album?.name || 'Shared photos', days, controller.signal)
        setJob(draft)
      }
      const result = await publish(draft, gallery, controller.signal, setProgress)
      const url = window.location.origin + result.url
      setShareUrl(url)
      setDeadline(result.expires_at)
      setShowShareBox(true)
      const ids = new Set(draft.manifest.photos.map(x => x.id))
      setGallery(prev => prev.map(x => ids.has(x.id) ? { ...x, shareLink: url } : x))
      await forgetDraft()
      setJob(null)
    } catch (e) {
      setError(controller.signal.aborted ? 'Upload paused.' : e.message)
    } finally {
      setUploading(false)
      active.current = null
    }
  }

  return (
    <div className="app">
      <header className="header">
        <Brand section="Workspace" />
        {!!gallery.length && <SearchField label="Search photos or tags" value={search} onChange={value => { setSearch(value); setPage(0) }} />}
        {!!gallery.length && <button className="btn add-more" onClick={() => picker.current?.click()}><Plus size={18} />Add photos</button>}
        <input ref={picker} type="file" accept="image/png,image/jpeg,image/webp,image/gif" multiple hidden
          onChange={e => { addFiles(e.target.files); e.target.value = '' }} />
      </header>

      <main className="main">
        {!gallery.length && <StudioIntro pick={() => picker.current?.click()} onDrop={handleDrop} onDragOver={handleDragOver} onDragLeave={handleDragLeave} />}
        <Lifetime value={job?.manifest.days ?? days} change={setDays} disabled={uploading || !!job} />
        {(!!gallery.length || job || shareUrl || uploading) && <div className="toolbar">
          <div className="toolbar-left"><h1 className="page-title">Your photos</h1><span className="photo-count">{filteredGallery.length}</span></div>
          <div className="toolbar-actions">
            <button className="btn primary" onClick={() => uploadToServer()} disabled={uploading || !restored || (!job && !gallery.length)}>
              <UploadSimple size={18} aria-hidden="true" />{uploading ? 'Uploading…' : job ? 'Resume ' + job.manifest.title : selectedCount ? 'Publish selected' : 'Publish new photos'}
            </button>
            {uploading && <button className="btn" onClick={() => active.current?.abort()}>Pause</button>}
            {job && !uploading && <button className="btn" onClick={async () => { try { await forgetDraft(); setJob(null); setError('') } catch (e) { setError(e.message) } }}>Discard local draft</button>}
            {shareUrl && (
              <button className="btn share" onClick={() => setShowShareBox(true)}>
                <ShareNetwork size={18} aria-hidden="true" />Share
              </button>
            )}
            <button className="btn danger" disabled={!gallery.length} onClick={clearAll}>
              <Trash size={18} aria-hidden="true" />Clear local workspace
            </button>
          </div>
        </div>}

        {error && <div role="alert" className="error show">{error}</div>}

        {showShareBox && shareUrl && (
          <ShareLinkBox url={shareUrl} deadline={deadline} onClose={() => setShowShareBox(false)} />
        )}

        {uploading && (
          <div className="upload-progress show">
            <span>{progress.name} · {Math.round(progress.current / 1024)} / {Math.round(progress.total / 1024)} KiB confirmed</span>
            <div className="progress-bar">
              <div className="progress-fill" style={{ transform: `scaleX(${progress.total ? progress.current / progress.total : 0})` }} />
            </div>
          </div>
        )}

        <div className="masonry">
          {visible.map((item) => (
            <PhotoCard
              key={item.id}
              item={item}
              hasSel={selectedCount > 0}
              onToggle={() => toggleSelect(item.id)}
              onDelete={() => deleteImg(item.id)}
              onMove={(dir) => moveImg(index.get(item.id).pos, dir)}
              onAddTag={(tag) => addTag(item.id, tag)}
              onRemoveTag={(tag) => removeTag(item.id, tag)}
            />
          ))}
        </div>

        {pages > 1 && <nav className="album-pages" aria-label="Workspace pages"><button disabled={!currentPage} onClick={() => setPage(currentPage - 1)}>Previous</button>
          {' '}Page {currentPage + 1} of {pages}{' '}
          <button disabled={currentPage + 1 === pages} onClick={() => setPage(currentPage + 1)}>Next</button></nav>}
        {!!gallery.length && !filteredGallery.length && <p>No matching photos</p>}
        {albums.length > 0 && (
          <div className="albums">
            <h2 className="section-title">Collections</h2>
            <div className="album-grid">
              {albums.map(album => (
                <AlbumCard
                  key={album.id}
                  album={album}
                  gallery={gallery}
                  onDelete={() => deleteAlbum(album.id)}
                  onPublish={() => uploadToServer(album)} disabled={uploading || !!job || !restored}
                />
              ))}
            </div>
          </div>
        )}
        <WorkspaceFooter />
      </main>

      {selectedCount > 0 && (
        <AnimatedBar className="sel-bar" initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }}>
          <span className="sel-count">{selectedCount} selected</span>
          <div className="divider" />
          <button className="sel-btn" onClick={() => selectItems(visible)}>
            <SelectionAll size={18} aria-hidden="true" />Select page
          </button>
          <button className="sel-btn" onClick={() => selectItems(filteredGallery)}>Select all matches</button>
          <button className="sel-btn" onClick={deselectAll}>
            <SelectionSlash size={18} aria-hidden="true" />Clear
          </button>
          <div className="divider" />
          <button className="sel-btn primary" onClick={() => setModalOpen(true)}>
            <FolderPlus size={18} aria-hidden="true" />Create Album
          </button>
        </AnimatedBar>
      )}

      {modalOpen && <AlbumDialog name={albumName} setName={setAlbumName} close={() => setModalOpen(false)} create={createAlbum} />}
    </div>
  )
}

function AlbumDialog({ name, setName, close, create }) {
  const ref = useRef(null)
  useEffect(() => {
    const dialog = ref.current
    dialog.showModal()
    return () => dialog.close()
  }, [])
  return <dialog ref={ref} className="studio-dialog" aria-labelledby="album-title" onCancel={close}
    onClick={e => { if (e.target === e.currentTarget) close() }}>
    <form className="modal" onSubmit={e => { e.preventDefault(); create() }}>
      <h2 id="album-title">New album</h2>
      <label>Album name<input autoFocus required className="modal-input" placeholder="Album name..." value={name} onChange={e => setName(e.target.value)} /></label>
      <div className="modal-actions"><button type="button" className="btn" onClick={close}>Cancel</button><button className="btn primary" disabled={!name.trim()}>Create</button></div>
    </form>
  </dialog>
}

function PhotoCard({ item, hasSel, onToggle, onDelete, onMove, onAddTag, onRemoveTag }) {
  const [tagInput, setTagInput] = useState('')

  const handleAddTag = () => {
    if (tagInput.trim()) {
      onAddTag(tagInput)
      setTagInput('')
    }
  }

  return (
    <div className={`photo${item.selected ? ' selected' : ''}`}>
      <img src={item.url} alt={item.name} loading="lazy" />
      <div className="photo-overlay">
        <div className="card-top">
          <button aria-pressed={item.selected} aria-label={`Select ${item.name}`}
            className={`checkbox${item.selected ? ' checked visible' : ''}${hasSel ? ' visible' : ''}`}
            onClick={onToggle}
          >
            {item.selected && <Check size={18} aria-hidden="true" />}
          </button>
          <div className="card-btns">
            <button className="card-btn" onClick={() => onMove(-1)} title="Move left">
              <CaretLeft size={18} aria-hidden="true" />
            </button>
            <button className="card-btn" onClick={() => onMove(1)} title="Move right">
              <CaretRight size={18} aria-hidden="true" />
            </button>
            <button className="card-btn del" onClick={onDelete} title="Delete">
              <Trash size={18} aria-hidden="true" />
            </button>
          </div>
        </div>
        <div className="card-bottom">
          <div className="photo-name">{item.name}</div>
          <div className="tag-row">
            <input
              aria-label={`Tags for ${item.name}`} className="tag-input"
              placeholder="Add tags..."
              value={tagInput}
              onChange={(e) => setTagInput(e.target.value)}
              onKeyPress={(e) => e.key === 'Enter' && handleAddTag()}
            />
            <button className="tag-add" onClick={handleAddTag}>Add</button>
          </div>
          {item.tags.length > 0 && (
            <div className="tags">
              {item.tags.map(tag => (
                <span key={tag} className="tag">
                  {tag}
                  <button aria-label={`Remove tag ${tag}`} className="tag-x" onClick={() => onRemoveTag(tag)}>×</button>
                </span>
              ))}
            </div>
          )}

        </div>
      </div>
    </div>
  )
}

function ShareLinkBox({ url, deadline, onClose }) {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    await navigator.clipboard.writeText(url)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  return (
    <div className="share-box">
      <div className="share-box-header">
        <LinkSimple size={18} aria-hidden="true" />
        <span className="share-box-title">Share album · Expires {new Date(deadline * 1000).toLocaleString()}</span>
        <button aria-label="Close share link" className="share-box-close" onClick={onClose}>
          <X size={18} aria-hidden="true" />
        </button>
      </div>
      <div className="share-box-body">
        <input className="share-box-input" value={url} readOnly onClick={e => e.target.select()} />
        <button className={`share-box-copy${copied ? ' copied' : ''}`} onClick={handleCopy}>
          {copied ? <Check size={18} /> : <Copy size={18} />}
          {copied ? 'Copied!' : 'Copy'}
        </button>
      </div>
    </div>
  )
}

function AlbumCard({ album, gallery, onDelete, onPublish, disabled }) {
  const images = album.ids.map(id => gallery.find(x => x.id === id)).filter(Boolean).slice(0, 4)

  return (
    <div className="album-card">
      <div className="album-cover">
        {[0, 1, 2, 3].map(i => (
          images[i]
            ? <img key={i} src={images[i].url} alt="" loading="lazy" />
            : <div key={i} className="ph"><ImageSquare size={18} aria-hidden="true" /></div>
        ))}
      </div>
      <div className="album-info">
        <div className="album-name">{album.name}</div>
        <div className="album-count">{album.ids.filter(id => gallery.find(x => x.id === id)).length} photos</div>
      </div>
      <div className="album-actions">
        <button className="btn" disabled={disabled || !images.length} onClick={onPublish}>Publish this album</button>
        <button className="btn" onClick={onDelete}>
          <Trash size={18} aria-hidden="true" />Remove
        </button>
      </div>
    </div>
  )
}

function GalleryViewer({ galleryId }) {
  const [images, setImages] = useState([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')

  useEffect(() => {
    const fetchGallery = async () => {
      try {
        const res = await fetch(`/api/gallery/${galleryId}`)
        if (!res.ok) throw new Error('Gallery not found')
        const data = await res.json()
        setImages(data.images || [])
      } catch (e) {
        setError(e.message)
      } finally {
        setLoading(false)
      }
    }
    fetchGallery()
  }, [galleryId])

  return (
    <div className="app">
      <header className="header">
        <a href="/" className="logo">
          <Images size={18} aria-hidden="true" />
          <h1>Kioku</h1>
        </a>
        <div className="viewer-header-right">
          <span className="photo-count">{images.length} photo{images.length !== 1 ? 's' : ''}</span>
          <a className="btn primary" href={`/api/gallery/${galleryId}/download`}>
            <DownloadSimple size={18} aria-hidden="true" />Download All
          </a>
        </div>
      </header>

      <main className="main">
        {loading && (
          <div className="empty">
            <Hourglass size={18} aria-hidden="true" />
            <h3>Loading gallery...</h3>
          </div>
        )}

        {error && <div role="alert" className="error show">{error}</div>}

        {!loading && !error && images.length === 0 && (
          <div className="empty">
            <ImageSquare size={18} aria-hidden="true" />
            <h3>Gallery is empty</h3>
            <p>No images found in this gallery</p>
          </div>
        )}

        {!loading && images.length > 0 && (
          <div className="masonry">
            {images.map((url, i) => (
              <div key={i} className="viewer-photo">
                <img src={url} alt={`Photo ${i + 1}`} loading="lazy" />
              </div>
            ))}
          </div>
        )}
      </main>
    </div>
  )
}

export default App
