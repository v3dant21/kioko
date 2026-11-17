import { motion, useReducedMotion } from 'motion/react'
import { Aperture, Images, Plus, ArrowUpRight, Clock, MagnifyingGlass } from '@phosphor-icons/react'
import mountain from './assets/mountain.webp'
import credit from "./assets/Mountain-LICENSE.txt?url"

const PhotoPrint = motion.figure

export function Brand({ section }) {
  return <div className="brand-line"><a className="studio-brand" href="/" aria-label="Kioku home"><Aperture size={33} weight="bold" />Kioku</a><span className="brand-section">{section}</span></div>
}

export function SearchField({ value, onChange, label }) {
  return <label className="studio-search"><MagnifyingGlass size={18} /><input type="search" aria-label={label} placeholder={label} value={value} onChange={e => onChange(e.target.value)} /></label>
}

export function StudioIntro({ pick, onDrop, onDragOver, onDragLeave }) {
  const reduced = useReducedMotion()
  return <section className="studio-intro" onDrop={onDrop} onDragOver={onDragOver} onDragLeave={onDragLeave}>
    <div className="intro-copy">
      <h1>Create a<br /><em>temporary</em> album.</h1>
      <p>Gather your photos, choose how long they stay online, and share them with one link.</p>
      <button className="studio-primary" onClick={pick}><Plus size={20} weight="bold" />Add photos</button>
      <span className="drop-hint">or drop them here</span>
    </div>
    <div className="print-stage">
      <PhotoPrint className="studio-print" initial={reduced ? false : { y: 14, rotate: 1, opacity: 0 }} animate={{ y: 0, rotate: -4, opacity: 1 }} transition={{ type: 'spring', stiffness: 130, damping: 21 }}>
        <img src={mountain} width="960" height="540" alt="A snow-capped mountain under a pale turquoise sky" />
        <figcaption><span>Mountain</span><a href={credit} download="Mountain-credits.txt">Andy Betts</a></figcaption>
      </PhotoPrint>
    </div>
  </section>
}

export function Lifetime({ value, change, disabled }) {
  return <section className="lifetime">
    <div className="setting-label"><Clock size={24} /><div><h2>Album lifetime</h2><p>Starts when you publish.</p></div></div>
    <fieldset disabled={disabled} className="lifetime-options"><legend className="sr-only">Album lifetime</legend>
      {[[1,'24 hours'],[3,'3 days'],[7,'7 days']].map(([n,label]) => <label key={n}><input type="radio" name="lifetime" checked={value === n} onChange={() => change(n)} /><span>{label}</span></label>)}
    </fieldset>
  </section>
}

export function WorkspaceFooter() {
  return <footer className="studio-footer"><span><Images size={16} />JPEG, PNG, WebP, GIF</span><span>32 MiB per photo</span></footer>
}

export function DownloadLink({ href }) {
  return <a className="studio-primary" href={href}>Download all as ZIP <ArrowUpRight size={18} /></a>
}
