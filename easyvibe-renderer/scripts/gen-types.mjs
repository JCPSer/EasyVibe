// 从 easyvibe-map-schema-v1.json 生成 TS 类型——单一事实源
// 用法：npm run gen:types（schema 变更后必须重跑，渲染器禁止手工镜像 schema 字段）
import { readFileSync, writeFileSync } from 'node:fs'
import { compile } from 'json-schema-to-typescript'

const schemaPath = new URL('../../easyvibe-map-schema-v1.json', import.meta.url)
const outPath = new URL('../src/types/generated.ts', import.meta.url)

const schema = JSON.parse(readFileSync(schemaPath, 'utf-8'))
const ts = await compile(schema, 'CodeMap', {
  bannerComment: `/* eslint-disable */
/**
 * 本文件由 scripts/gen-types.mjs 从 easyvibe-map-schema-v1.json 自动生成，请勿手改。
 * 修改数据格式请改 Schema，然后 npm run gen:types。
 */`,
  additionalProperties: false,
  strictIndexSignatures: true,
})
writeFileSync(outPath, ts)
console.log('generated.ts 生成完成')
