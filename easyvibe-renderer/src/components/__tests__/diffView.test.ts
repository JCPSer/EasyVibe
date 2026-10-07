// diffView 纯函数单测：行分类 / 文本解析 / 增删统计。
import { describe, expect, it } from 'vitest'
import { classifyDiffLine, countDiffStats, parseDiffText } from '../git/diffView'

describe('diffView · classifyDiffLine', () => {
  it('文件头一族弱化显示', () => {
    for (const l of [
      'diff --git a/x.ts b/x.ts',
      'index 111..222 100644',
      '--- a/x.ts',
      '+++ b/x.ts',
      'similarity index 90%',
      'rename from a',
      'rename to b',
      'old mode 100644',
      'new mode 100755',
    ]) {
      expect(classifyDiffLine(l), l).toBe('file')
    }
  })
  it('hunk 头 / 新增 / 删除 / 上下文 / meta', () => {
    expect(classifyDiffLine('@@ -1,2 +1,3 @@')).toBe('hunk')
    expect(classifyDiffLine('+新增行')).toBe('add')
    expect(classifyDiffLine('-删除行')).toBe('del')
    expect(classifyDiffLine(' 上下文')).toBe('context')
    expect(classifyDiffLine('普通行')).toBe('context')
    expect(classifyDiffLine('\\ No newline at end of file')).toBe('meta')
  })
  it('未跟踪合成 diff 的头行分类正确', () => {
    expect(classifyDiffLine('--- /dev/null')).toBe('file')
    expect(classifyDiffLine('+++ b/new.txt')).toBe('file')
    expect(classifyDiffLine('@@ -0,0 +1,2 @@')).toBe('hunk')
  })
})

describe('diffView · parseDiffText', () => {
  it('整体解析 + 尾部换行不产生空行', () => {
    const text = '--- a/x.ts\n+++ b/x.ts\n@@ -1 +1 @@\n-old\n+new\n'
    const lines = parseDiffText(text)
    expect(lines).toHaveLength(5)
    expect(lines.map((l) => l.kind)).toEqual(['file', 'file', 'hunk', 'del', 'add'])
    expect(lines[3].text).toBe('-old')
  })
  it('空文本 → 空序列', () => {
    expect(parseDiffText('')).toEqual([])
  })
})

describe('diffView · countDiffStats', () => {
  it('统计 +/- 行', () => {
    const lines = parseDiffText('--- a\n+++ b\n@@ -1,2 +1,3 @@\n-a\n+x\n+y\n ctx\n')
    expect(countDiffStats(lines)).toEqual({ adds: 2, dels: 1 })
  })
})
