// src/api/** 汇总出口（仅 re-export，不聚合 fetch；唯一 fetch 出口是 ./core）。
export * as core from './core'
export * as repos from './repos'
export * as git from './git'
export * as canvas from './canvas'
export * as chat from './chat'
export * as task from './task'
export * as settings from './settings'
export * as system from './system'
