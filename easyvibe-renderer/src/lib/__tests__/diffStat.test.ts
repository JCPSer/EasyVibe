import { describe, expect, it } from 'vitest'
import { absTime, aggregateByModule, moduleOfFile, parseDiffStat, relTime } from '@/lib/diffStat'

const SAMPLE = ` src/App.tsx                | 12 ++++++-------
 src/components/Node.tsx    |  5 ++---
 package.json               |  2 +-
 3 files changed, 9 insertions(+), 10 deletions(-)`

describe('parseDiffStat', () => {
  it('解析文件行与汇总行', () => {
    const s = parseDiffStat(SAMPLE)
    expect(s.files).toHaveLength(3)
    expect(s.files[0]).toEqual({ path: 'src/App.tsx', adds: 6, dels: 7 })
    expect(s.files[1]).toEqual({ path: 'src/components/Node.tsx', adds: 2, dels: 3 })
    expect(s.fileCount).toBe(3)
    expect(s.insertions).toBe(9)
    expect(s.deletions).toBe(10)
  })

  it('二进制文件行无 +/- 号，按 0 增删跳过汇总', () => {
    const s = parseDiffStat(' assets/logo.png | Bin 0 -> 12 bytes\n 1 file changed, 0 insertions(+), 0 deletions(-)')
    expect(s.files).toHaveLength(0)
    expect(s.fileCount).toBe(1)
  })

  it('无汇总行时从文件行累计', () => {
    const s = parseDiffStat(' a.ts | 3 ++-\n b.ts | 2 +')
    expect(s.fileCount).toBe(2)
    expect(s.insertions).toBe(3)
    expect(s.deletions).toBe(1)
  })

  it('空文本不炸', () => {
    const s = parseDiffStat('')
    expect(s.files).toHaveLength(0)
    expect(s.fileCount).toBe(0)
  })
})

describe('aggregateByModule', () => {
  const modules = [
    { id: 'core', name: '核心', files: ['src/core/**'] },
    { id: 'ui', name: '界面', files: ['src/ui'] },
  ]

  it('按 glob 前缀聚合，未命中归入未映射', () => {
    const agg = aggregateByModule(
      [
        { path: 'src/core/a.ts', adds: 10, dels: 2 },
        { path: 'src/core/b.ts', adds: 3, dels: 0 },
        { path: 'src/ui/Button.tsx', adds: 1, dels: 1 },
        { path: 'README.md', adds: 1, dels: 0 },
      ],
      modules,
    )
    const core = agg.find((a) => a.id === 'core')!
    expect(core.adds).toBe(13)
    expect(core.dels).toBe(2)
    expect(core.fileCount).toBe(2)
    expect(agg.find((a) => a.id === '_other')!.name).toBe('未映射文件')
    expect(agg[0].id).toBe('core')
  })

  it('R2 边界：前缀必须有路径段边界（core 不得匹配 coreography）', () => {
    const mods = [{ id: 'core', name: '核心', files: ['src/core/**'] }]
    expect(moduleOfFile('src/core/a.ts', mods)?.id).toBe('core')
    expect(moduleOfFile('lib/src/core/deep/x.ts', mods)?.id).toBe('core')
    expect(moduleOfFile('src/coreography/data.ts', mods)).toBeNull()
    expect(moduleOfFile('src/core_plus/x.ts', mods)).toBeNull()
    expect(aggregateByModule([{ path: 'src/coreography/d.ts', adds: 1, dels: 0 }], mods)[0].id).toBe('_other')
  })
})

describe('时间格式化', () => {
  const now = Date.parse('2026-10-01T12:00:00')

  it('relTime 分级', () => {
    expect(relTime('2026-10-01T11:59:30', now)).toBe('刚刚')
    expect(relTime('2026-10-01T11:45:00', now)).toBe('15 分钟前')
    expect(relTime('2026-10-01T09:00:00', now)).toBe('3 小时前')
    expect(relTime('2026-09-26T12:00:00', now)).toBe('5 天前')
    expect(relTime(null, now)).toBe('—')
    expect(relTime('not-a-date', now)).toBe('not-a-date')
  })

  it('absTime 格式化', () => {
    expect(absTime('2026-09-30T09:15:22')).toBe('2026-09-30 09:15')
    expect(absTime(undefined)).toBe('—')
  })

  it('epoch 秒/毫秒串也能解析（巡检 started_at 落库口径）', () => {
    expect(absTime('1790871442')).toBe('2026-10-02 00:17')
    expect(relTime('1790871442', Date.parse('2026-10-02T01:17:22'))).toBe('1 小时前')
    expect(relTime('1790871442000', Date.parse('2026-10-02T01:17:22'))).toBe('1 小时前')
    expect(absTime('1790871442')).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/)
  })
})
