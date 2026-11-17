// How a lesson file reaches the AI, shown wherever files are chosen or listed. The table comes from the core
// (files.rs), so the app never claims a file is sent when the core would send text.
import { useEffect, useState, type ReactElement } from 'react'
import type { Doc, FileDelivery, FileSupport, FileSupportTable } from '../../../shared/types'
import { api } from '../api'
import { Icon } from '../icons'
import { useApp } from '../state'

/** The support table for the current provider and model, refetched when either changes. */
export function useFileSupport(): FileSupportTable | null {
  const { settings } = useApp()
  const [table, setTable] = useState<FileSupportTable | null>(null)
  const key = `${settings?.ai.provider}|${settings?.ai.model}|${settings?.ai.baseUrls?.gemini ?? ''}`
  useEffect(() => {
    let live = true
    const load = (): void => { api.fileSupport().then((t) => { if (live) setTable(t) }, () => { if (live) setTable(null) }) }
    load()
    // A model can turn a file type down mid-run, and settings can change elsewhere: refresh with the data.
    const off = api.on('data', load)
    return () => { live = false; off() }
  }, [key])
  return table
}

const NAMES: Record<string, string> = { pdf: 'PDF', docx: 'Word', md: 'Markdown', txt: 'Text', png: 'Pictures', jpg: 'JPEG', webp: 'WebP' }
export const typeName = (ext: string): string => NAMES[ext] ?? ext.toUpperCase()

const mb = (b: number): number => Math.round(b / 1_048_576)

export interface Delivery { kind: FileDelivery; label: string; detail: string }

/** How this lesson goes to the model now: its type's row, adjusted for this file's size, pages and history. */
export function deliveryFor(d: Pick<Doc, 'ext' | 'fileBytes' | 'pages' | 'readByAi' | 'needsReading' | 'fileHash'>, table: FileSupportTable | null): Delivery | null {
  if (!table || d.ext === 'manual') return null
  const ext = d.ext === 'jpeg' ? 'jpg' : d.ext === 'markdown' ? 'md' : d.ext
  const row = table.types.find((t) => t.ext === ext)
  if (!row) return null
  if (row.kind !== 'file') {
    if (d.needsReading) return { kind: 'unsupported', label: 'Can’t be read', detail: `${row.detail} The Mac found no text in this file.` }
    return row
  }
  if (d.readByAi && !d.needsReading) return { kind: 'text', label: 'Read by AI', detail: 'The AI wrote out this lesson’s text from the file; organizing uses that text.' }
  if (!d.fileHash) return { kind: 'extracted', label: 'Text extracted', detail: 'Imported before files could be sent. Import the file again to send the file itself.' }
  if (row.maxBytes && (d.fileBytes ?? 0) > row.maxBytes) return { kind: 'extracted', label: 'Text extracted', detail: `This file is ${mb(d.fileBytes ?? 0)} MB; files over ${mb(row.maxBytes)} MB go as extracted text.` }
  if (row.maxPages && (d.pages ?? 0) > row.maxPages) return { kind: 'extracted', label: 'Text extracted', detail: `This PDF has ${d.pages} pages; over ${row.maxPages}, ${table.provider} gets the extracted text.` }
  if (d.needsReading && d.ext === 'pdf' && (d.pages ?? 0) > 40) return { kind: 'unsupported', label: 'Too long to read', detail: `This scan has ${d.pages} pages; the AI writes out at most 40 at a time. Split it into shorter PDFs.` }
  if (d.needsReading) return { ...row, label: 'AI reads it', detail: `The Mac found no text here (a scan or a picture), so ${table.provider} reads the file and writes out its text first.` }
  return row
}

const GLYPH: Record<FileDelivery, (p: { size?: number }) => ReactElement> = {
  file: Icon.file, text: Icon.fileText, extracted: Icon.fileText, unsupported: Icon.close
}

/** A quiet inline status: glyph and words, with the reason on hover and for VoiceOver. */
export function DeliveryTag({ d, compact }: { d: Delivery; compact?: boolean }): ReactElement {
  const G = GLYPH[d.kind]
  return (
    <span className={`delivery ${d.kind}`} title={d.detail} aria-label={`${d.label}. ${d.detail}`}>
      <G size={13} />{!compact && <span>{d.label}</span>}
    </span>
  )
}

/** One sentence for the drop target: what the current model reads as files. */
export function dropSummary(table: FileSupportTable | null): string {
  if (!table) return 'PDF, Word, Markdown, text, and pictures for Gemini or the scheduled task'
  const kinds = (k: FileDelivery): string[] => [...new Set(table.types.filter((t) => t.kind === k).map((t) => (['png', 'jpg', 'webp'].includes(t.ext) ? 'pictures' : typeName(t.ext))))]
  const files = kinds('file'), extracted = kinds('extracted')
  const list = (l: string[]): string => l.length < 2 ? l.join('') : `${l.slice(0, -1).join(', ')} and ${l[l.length - 1]}`
  if (!files.length) return `${table.provider} gets text only: Markdown and text as written, ${list(extracted)} as extracted text`
  return `${table.provider} reads ${list(files)} as they are${extracted.length ? `; ${list(extracted)} go as extracted text` : ''}`
}

/** Settings: every type and how it reaches the current model. */
export function FileSupportList({ table }: { table: FileSupportTable }): ReactElement {
  // Picture formats always share one answer, so they share one row.
  const pics = ['png', 'jpg', 'webp']
  const rows: (FileSupport & { exts: string[] })[] = []
  for (const t of table.types) {
    const pic = pics.includes(t.ext)
    const same = pic ? rows.find((r) => pics.includes(r.ext) && r.kind === t.kind && r.detail === t.detail) : undefined
    if (same) same.exts.push(t.ext)
    else rows.push({ ...t, exts: [t.ext] })
  }
  return (
    <table className="table filetypes">
      <thead><tr><th>File</th><th>How {table.model || table.provider} gets it</th></tr></thead>
      <tbody>
        {rows.map((t) => (
          <tr key={t.ext}>
            <td className="ft-name">{t.exts.length > 1 ? 'Pictures' : typeName(t.ext)} <span className="muted">{t.exts.map((e) => `.${e}`).join(' ')}</span></td>
            <td><DeliveryTag d={t} /><div className="help">{t.detail}{t.maxBytes ? ` Up to ${mb(t.maxBytes)} MB${t.maxPages ? ` and ${t.maxPages} pages` : ''}.` : ''}</div></td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}
