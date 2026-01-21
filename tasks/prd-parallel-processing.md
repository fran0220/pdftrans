# PRD: 并行处理与任务管理优化

## Introduction

优化 PDF 翻译器：所有页面全并行处理（最多20页），支持多任务，SSE断开自动取消。

## Goals

- 全页面并行：所有页同时 OCR + 翻译（最多20页）
- 最多 3 个并发任务，超出返回错误
- SSE 断开自动取消任务
- 前端显示当前任务列表

## User Stories

### US-001: 全页面并行处理
**Description:** 所有页面同时处理 OCR 和翻译。

**Acceptance Criteria:**
- [ ] 每页创建独立 tokio task 并发执行
- [ ] 每页任务：OCR → 翻译（顺序）
- [ ] 最多支持 20 页并行，超出分批处理
- [ ] 按页码顺序收集结果生成 PDF
- [ ] `cargo build` 成功

### US-002: 限制并发任务数
**Description:** 最多 3 个任务同时运行。

**Acceptance Criteria:**
- [ ] 第 4 个任务返回 429 错误
- [ ] 提示"服务繁忙，请稍后重试"
- [ ] `cargo build` 成功

### US-003: SSE 断开自动取消
**Description:** 页面关闭时取消后台任务。

**Acceptance Criteria:**
- [ ] SSE 连接断开触发取消
- [ ] 所有页任务停止执行
- [ ] `cargo build` 成功

### US-004: 前端任务列表
**Description:** 显示所有进行中的任务。

**Acceptance Criteria:**
- [ ] 添加 `/tasks` API
- [ ] 前端显示任务卡片（文件名、进度）
- [ ] 点击切换查看详情

### US-005: 并行进度显示
**Description:** 显示并行处理进度。

**Acceptance Criteria:**
- [ ] 显示 "OCR: 5/17, 翻译: 3/17"
- [ ] 整体进度正确计算

## Functional Requirements

- FR-1: 每页独立 task 全并行（最多20页）
- FR-2: 全局最多 3 个任务
- FR-3: SSE 断开取消任务
- FR-4: `/tasks` 返回任务列表

## Non-Goals

- 不暴露并发配置
- 不持久化任务
- 不支持断点续传

## Success Metrics

- 17 页处理时间减少 70%+
- 页面关闭后任务立即停止
