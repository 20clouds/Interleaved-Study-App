import { describe, expect, it } from 'vitest'
import { prepare } from '../src/main/preprocess'
import { ambiguousPairs, cacheKey, finalizeWindows, normalizeWindow, topicExcerpts, windowCacheContent } from '../src/main/organize'
import type { Para } from '../src/shared/types'
import { extractJson, InvalidJson } from '../src/main/ai/json'

const paras = prepare(Array.from({ length: 20 }, (_, i) => `Paragraph ${i + 1} explains osmosis and diffusion across the membrane in detail number ${i}. ${'More words here. '.repeat(10)}`).join('\n\n')).paras

describe('extractJson', () => {
  it('strips code fences and prose around JSON', () => {
    expect(extractJson('```json\n{"a":1}\n```')).toEqual({ a: 1 })
    expect(extractJson('Sure! Here it is: {"topics":[{"start":1}]} hope that helps')).toEqual({ topics: [{ start: 1 }] })
    expect(extractJson('[1,2,3,]')).toEqual([1, 2, 3])
  })
  it('throws InvalidJson for garbage', () => {
    expect(() => extractJson('no json here')).toThrow(InvalidJson)
  })
})

describe('normalizeWindow: validation and local repair', () => {
  it('clamps, removes overlaps and fills gaps so ranges cover the window exactly', () => {
    const raw = { topics: [
      { start: 3, end: 6, name: 'Osmosis', unit: 'Transport' },
      { start: 5, end: 9, name: 'Diffusion' }, // overlaps
      { start: 12, end: 99, name: 'Active transport' } // gap 10-11, beyond window
    ] }
    const r = normalizeWindow(raw, 1, 15, paras, 'Lesson')
    expect(r.topics.map((t) => [t.start, t.end])).toEqual([[1, 6], [7, 11], [12, 15]])
  })
  it('replaces generic names and accepts p-prefixed ids', () => {
    const r = normalizeWindow({ topics: [{ start: 'p1', end: 'p4', name: 'Part 1' }, { start: 'p5', end: 'p8', name: 'Section 2' }] }, 1, 8, paras, 'Lesson')
    for (const t of r.topics) expect(t.name).not.toMatch(/^(Part|Section) \d/)
    expect(r.topics[0].name).toMatch(/^Paragraph 1 explains/)
  })
  it('drops contained topics and caps lists', () => {
    const r = normalizeWindow({ topics: [{ start: 1, end: 8, name: 'Big', terms: ['a', 'b', 'c', 'd', 'e', 'f', 'g'] }, { start: 2, end: 3, name: 'Inside', terms: ['z'] }] }, 1, 8, paras, 'L')
    expect(r.topics).toHaveLength(1)
    expect(r.topics[0].terms.length).toBeLessThanOrEqual(6)
  })
  it('throws for unusable output so the queue can do a repair retry', () => {
    expect(() => normalizeWindow({ nope: true }, 1, 5, paras, 'L')).toThrow(InvalidJson)
    expect(() => normalizeWindow({ topics: [{ name: 'x' }] }, 1, 5, paras, 'L')).toThrow(InvalidJson)
  })
  it('keeps a proposed title but not a generic one', () => {
    expect(normalizeWindow({ title: 'Membrane transport', topics: [{ start: 1, end: 5, name: 'Osmosis' }] }, 1, 5, paras, 'L').title).toBe('Membrane transport')
    expect(normalizeWindow({ title: 'Part 2', topics: [{ start: 1, end: 5, name: 'Osmosis' }] }, 1, 5, paras, 'L').title).toBeNull()
  })
})

describe('finalizeWindows: merge logic', () => {
  it('merges a continuing topic across windows and same-name neighbours', () => {
    const w1 = normalizeWindow({ topics: [{ start: 1, end: 5, name: 'Osmosis' }, { start: 6, end: 10, name: 'Diffusion rates' }] }, 1, 10, paras, 'L')
    const w2 = normalizeWindow({ topics: [{ start: 11, end: 14, name: 'Diffusion (continued)', continues: true }, { start: 15, end: 20, name: 'Active transport' }] }, 11, 20, paras, 'L')
    const f = finalizeWindows([w1, w2], paras, 'L')
    expect(f.map((t) => t.name)).toEqual(['Osmosis', 'Diffusion rates', 'Active transport'])
    expect(f[1].start).toBe(6)
    expect(f[1].end).toBe(14)
  })
  it('merges tiny topics into a neighbour and fills units', () => {
    const w = normalizeWindow({ topics: [{ start: 1, end: 9, name: 'Osmosis', unit: 'Transport' }, { start: 10, end: 10, name: 'Tiny bit' }, { start: 11, end: 20, name: 'Pumps' }] }, 1, 20, paras, 'L')
    const tinyParas = paras.map((p) => (p.id === 10 ? { ...p, words: 5 } : p))
    const f = finalizeWindows([w], tinyParas, 'L')
    expect(f.map((t) => t.name)).toEqual(['Osmosis', 'Pumps'])
    expect(f.every((t) => t.unit === 'Transport')).toBe(true)
  })
  it('flags adjacent similar names as ambiguous', () => {
    const pairs = ambiguousPairs([{ name: 'Enzyme kinetics basics', unit: 'u' }, { name: 'Enzyme kinetics models', unit: 'u' }, { name: 'Cell walls', unit: 'u' }])
    expect(pairs).toEqual([[0, 1]])
  })
})

describe('cache keys', () => {
  it('depend on content and prompt version, not the model by default', () => {
    const c = windowCacheContent({ outline: 'o', paras, from: 1, to: 5, first: true })
    expect(cacheKey('organize', c, 'gemma-4-31b-it', false)).toBe(cacheKey('organize', c, 'gemini-3.8-flash', false))
    expect(cacheKey('organize', c, 'gemma-4-31b-it', true)).not.toBe(cacheKey('organize', c, 'gemini-3.8-flash', true))
    const c2 = windowCacheContent({ outline: 'o', paras, from: 1, to: 6, first: true })
    expect(cacheKey('organize', c, 'm', false)).not.toBe(cacheKey('organize', c2, 'm', false))
    expect(cacheKey('quiz', c, 'm', false)).not.toBe(cacheKey('organize', c, 'm', false))
  })
})

describe('topic excerpts', () => {
  const ps: Para[] = [
    { id: 1, text: 'Intro to equilibrium. Reactions go both ways.', words: 7 },
    { id: 2, text: 'More on rates. Now, the principle of microscopic reversibility says every step reverses.', words: 13 },
    { id: 3, text: 'Reversibility arrows are drawn as double harpoons.', words: 7 },
    { id: 4, text: 'Le Chatelier: a system shifts to undo a change.', words: 9 }
  ]
  const fold = (s: string): string => s.replace(/\s+/g, ' ').trim()

  it('cuts at the quoted opening words, even partway through the paragraph before', () => {
    const ex = topicExcerpts(ps, [
      { start: 1, end: 2, from: 'Intro to equilibrium. Reactions go' },
      { start: 3, end: 3, from: 'now the principle of microscopic reversibility' }, // case and punctuation differ
      { start: 4, end: 4, from: 'Le Chatelier: a system shifts' }
    ])
    expect(ex).toEqual([
      'Intro to equilibrium. Reactions go both ways.\n\nMore on rates.',
      'Now, the principle of microscopic reversibility says every step reverses.\n\nReversibility arrows are drawn as double harpoons.',
      'Le Chatelier: a system shifts to undo a change.'
    ])
    expect(fold(ex.join(' '))).toBe(fold(ps.map((p) => p.text).join(' ')))
  })

  it('falls back to the paragraph edge when the quote is missing, far away or reworded', () => {
    const ex = topicExcerpts(ps, [
      { start: 1, end: 2, from: '' },
      { start: 3, end: 3, from: 'Intro to equilibrium. Reactions go' }, // only in paragraph 1: too far back
      { start: 4, end: 4, from: 'Completely different words that are not there' }
    ])
    expect(ex).toEqual([ps[0].text + '\n\n' + ps[1].text, ps[2].text, ps[3].text])
  })

  it('keeps a heading or list marker in front of the quoted words', () => {
    const md: Para[] = [{ id: 1, text: 'Intro words here today.', words: 4 }, { id: 2, text: '## Reaction quotient basics', words: 3, heading: { level: 2, text: 'Reaction quotient basics' } }, { id: 3, text: 'Q compares the current mix.', words: 5 }]
    expect(topicExcerpts(md, [{ start: 1, end: 1, from: 'Intro words' }, { start: 2, end: 3, from: 'Reaction quotient basics' }])[1]).toBe('## Reaction quotient basics\n\nQ compares the current mix.')
  })

  it('accepts a quote whose tail the model tidied, by its first words', () => {
    const ex = topicExcerpts(ps, [{ start: 1, end: 2, from: 'x' }, { start: 3, end: 4, from: 'Now, the principle of microscopic reversibility states that' }])
    expect(ex[1].startsWith('Now, the principle')).toBe(true)
  })
})
