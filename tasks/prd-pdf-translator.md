# PRD: PDF 中文翻译器

## Introduction

一个简单的 Web 应用，用于将 PDF 文件翻译成中文。用户上传 PDF 后，系统调用 GPT-5 API 进行翻译，页面实时显示翻译进度。仅供个人使用，保持简单。

## Goals

- 用户能上传 PDF 文件
- 调用 GPT-5 API 将 PDF 内容翻译为中文
- 实时显示翻译进度（百分比/当前页数）
- 翻译完成后可下载结果

## User Stories

### US-001: 项目初始化与配置
**Description:** 作为开发者，我需要搭建 Rust Web 项目框架和环境配置。

**Acceptance Criteria:**
- [ ] 使用 Actix-web 或 Axum 创建 Web 服务
- [ ] 配置环境变量读取 (BASE_URL, API_KEY)
- [ ] 服务在 localhost:3000 启动
- [ ] `cargo build` 成功

### US-002: PDF 文件上传
**Description:** 作为用户，我想上传 PDF 文件以便进行翻译。

**Acceptance Criteria:**
- [ ] 页面有文件上传按钮，仅接受 .pdf 文件
- [ ] 文件上传后显示文件名
- [ ] 上传成功后触发翻译流程
- [ ] 在浏览器中验证上传功能

### US-003: PDF 文本提取
**Description:** 作为系统，我需要从 PDF 中提取文本内容。

**Acceptance Criteria:**
- [ ] 使用 pdf-extract 或 lopdf 提取 PDF 文本
- [ ] 按页面分割文本
- [ ] 处理基本的中英文 PDF

### US-004: GPT-5 翻译集成
**Description:** 作为系统，我需要调用 GPT-5 API 进行翻译。

**Acceptance Criteria:**
- [ ] 使用配置的 BASE_URL: http://67.230.171.248:8317/
- [ ] 使用配置的 API_KEY: sk-123456
- [ ] 调用 chat/completions 接口，模型使用 gpt-5
- [ ] 逐页发送翻译请求
- [ ] 正确处理 API 响应

### US-005: 实时进度显示
**Description:** 作为用户，我想看到翻译进度以了解处理状态。

**Acceptance Criteria:**
- [ ] 使用 SSE (Server-Sent Events) 推送进度
- [ ] 显示当前页数/总页数
- [ ] 显示进度百分比
- [ ] 翻译完成时显示完成状态
- [ ] 在浏览器中验证进度显示

### US-006: 翻译结果展示与下载
**Description:** 作为用户，我想查看和下载翻译结果。

**Acceptance Criteria:**
- [ ] 页面显示翻译后的中文内容预览
- [ ] 生成 PDF 文件，保持原文的基本格式（标题、段落、列表）
- [ ] 提供下载 PDF 文件的按钮
- [ ] 在浏览器中验证下载功能

## Functional Requirements

- FR-1: Web 服务使用 Rust (Axum) 框架
- FR-2: 前端使用内嵌 HTML，无需额外框架
- FR-3: 环境变量配置 `BASE_URL=http://67.230.171.248:8317/` 和 `API_KEY=sk-123456`
- FR-4: POST /upload 接收 PDF 文件
- FR-5: GET /progress/{task_id} 返回 SSE 流
- FR-6: GET /download/{task_id} 下载翻译后的 PDF 文件
- FR-7: 翻译 Prompt: "请将以下内容翻译成中文，保持原文格式（标题、段落、列表等）：{content}"
- FR-8: 生成 PDF 时保留原文的基本布局结构

## Non-Goals

- 不做用户认证系统
- 不做多用户并发处理
- 不做数据库持久化
- 不做复杂排版（图片、表格等复杂元素）
- 不做复杂的 UI 样式

## Technical Considerations

- **框架:** Axum (轻量、现代)
- **PDF 解析:** pdf-extract crate
- **HTTP 客户端:** reqwest
- **PDF 生成:** genpdf 或 printpdf crate（支持中文字体）
- **进度推送:** SSE (Server-Sent Events)
- **前端:** 内嵌 HTML + 原生 JavaScript

## Success Metrics

- 上传 PDF 后 3 秒内开始显示进度
- 翻译完成后能成功下载结果
- 单文件处理流程顺畅

## Open Questions

- 无（需求已明确）
