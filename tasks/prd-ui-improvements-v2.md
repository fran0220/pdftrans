# PRD: PDF翻译器 UI 改进 V2

## Introduction

对 PDF 多语言翻译器进行三项核心改进：移除历史任务显示简化界面、增加断线重连和错误恢复机制提升稳定性、添加右侧滑出日志面板提供详细处理信息。

## Goals

- 简化 UI，移除历史任务列表，每次刷新从干净状态开始
- 提供 SSE 断线自动重连和 API 失败自动重试机制
- 支持错误后从断点继续处理，无需重新上传
- 右侧滑出抽屉显示详细日志，包含每页处理时间和结果摘要

## User Stories

### US-001: 移除历史任务列表 ✅
**Description:** 作为用户，我希望页面简洁，刷新后不显示历史任务列表。

**Acceptance Criteria:**
- [x] 移除前端 `taskList` 元素和相关 DOM 操作
- [x] 移除页面加载时的 `/tasks` API 调用
- [x] 保留当前会话中正在处理的单个任务显示
- [x] 后端 `/tasks` API 可保留（供未来使用）但前端不调用
- [x] Typecheck/build 通过

### US-002: SSE 断线自动重连 ✅
**Description:** 作为用户，当网络断开后恢复时，我希望自动重新连接进度流。

**Acceptance Criteria:**
- [x] EventSource 断开时自动尝试重连（最多 5 次）
- [x] 重连间隔采用指数退避：1s, 2s, 4s, 8s, 16s
- [x] 重连时显示 "连接中..." 提示
- [x] 重连成功后恢复正常进度显示
- [x] 超过重连次数后显示 "连接失败，点击重试" 按钮
- [x] Typecheck/build 通过

### US-003: API 调用失败自动重试 ✅
**Description:** 作为开发者，我希望 OCR 和翻译 API 调用失败时自动重试。

**Acceptance Criteria:**
- [x] `translate.rs` 中的 `recognize_text` 和 `translate_text` 支持重试
- [x] 重试次数：3 次
- [x] 重试间隔：1s, 2s, 4s（指数退避）
- [x] 仅对网络错误和 5xx 错误重试，4xx 错误不重试
- [x] 日志记录每次重试信息
- [x] Typecheck/build 通过

### US-004: 错误后断点续传支持 ✅
**Description:** 作为用户，当处理失败时，我希望能从断点继续而非重新开始。

**Acceptance Criteria:**
- [x] `TaskData` 保存 `pdf_bytes: Option<Arc<Vec<u8>>>` 用于重试
- [x] `TaskData` 保存 `completed_pages: HashMap<usize, String>` 已翻译内容
- [x] 新增 `/retry/{task_id}` POST API，从失败点继续处理
- [x] `/retry` 仅允许 Error 状态且非 cancelled 的任务
- [x] `/retry` 加状态锁防止并发重复调用
- [x] 前端错误状态显示"重试"按钮，调用 retry API
- [x] 重试时跳过 `completed_pages` 中已存在的页面
- [x] 重试后进度从断点继续计算（ocr_done/translate_done 初始化为已完成数）
- [x] 任务完成后释放 `pdf_bytes` 节省内存
- [x] 任务过期（1h）后 retry 返回明确错误"任务已过期"
- [x] Typecheck/build 通过

### US-005: 右侧滑出日志抽屉 ✅
**Description:** 作为用户，我希望点击按钮展开右侧日志面板查看详细处理信息。

**Acceptance Criteria:**
- [x] 添加"查看日志"图标按钮在进度区域
- [x] 点击后从右侧滑出 300px 宽度的日志面板
- [x] 面板可点击关闭按钮或点击遮罩层关闭
- [x] 动画过渡平滑（0.3s ease）
- [x] 移动端响应式：宽度 100%
- [x] Typecheck/build 通过

### US-006: 详细日志内容增强 ✅
**Description:** 作为用户，我希望看到每页处理的详细时间和结果摘要。

**Acceptance Criteria:**
- [x] 新增 `PageSummary` 结构：`page_num`, `ocr_duration_ms`, `translate_duration_ms`, `ocr_chars`, `translated_chars`, `status`, `error`
- [x] `TaskProgress` 新增 `page_summaries: Vec<PageSummary>` 字段（每页一条汇总）
- [x] `logs` 保留最近 50 条系统/错误事件，不含每页详细事件
- [x] 日志面板分两区：上方为每页汇总表格，下方为系统日志
- [x] 每页汇总显示：页码、OCR 耗时、翻译耗时、字符数、状态
- [x] 错误时显示详细错误信息（红色高亮）
- [x] Typecheck/build 通过

## Functional Requirements

- FR-1: 移除 `index.html` 中 `#taskList` 元素及相关 CSS/JS 代码
- FR-2: 移除页面加载时 `fetch('/tasks')` 调用
- FR-3: EventSource 添加 `onerror` 处理，实现指数退避重连（1,2,4,8,16s）
- FR-4: 维护 `connectAttempt`、`reconnectTimer`、`isManuallyClosed` 状态机变量
- FR-5: 重连超过 5 次后停止，显示手动重试按钮；监听 `window.online` 触发立即重连
- FR-6: 识别"任务不存在"响应时停止重连（避免无意义重试）
- FR-7: `translate.rs` 添加 `with_retry` 包装函数，支持 3 次重试 + 指数退避 + jitter
- FR-8: 仅对 `reqwest::Error` 网络错误和 HTTP 5xx 状态码重试；4xx 不重试（429 可配置）
- FR-9: 每次 API 调用设置 30s timeout，防止挂死请求
- FR-10: `TaskData` 新增 `pdf_bytes: Option<Arc<Vec<u8>>>` 保存原始输入
- FR-11: `TaskData` 新增 `completed_pages: HashMap<usize, String>` 存储已翻译内容
- FR-12: `TaskData` 新增 `is_retrying: bool` 标记防止并发重试
- FR-13: 新增 `POST /retry/{task_id}` 路由，检查状态后调用 `retry_task` 函数
- FR-14: `retry_task` 从 `pdf_bytes` 重新渲染页面，跳过 `completed_pages` 中已有页
- FR-15: 任务完成后清空 `pdf_bytes` 节省内存
- FR-16: 前端添加右侧滑出抽屉组件，`position:fixed; transform:translateX()` 动画
- FR-17: 抽屉宽度桌面 300px，移动端 100%；overlay 点击/Esc 关闭
- FR-18: 新增 `PageSummary` 结构存储每页处理详情
- FR-19: `TaskProgress.page_summaries` 每页一条汇总，`logs` 限制 50 条系统事件
- FR-20: SSE 发送频率调整为 500ms，仅在有变化时推送

## Non-Goals

- 不实现多任务并行 UI（保持单任务显示）
- 不实现任务历史持久化到数据库
- 不实现单页重试（只支持整体断点续传）
- 不实现日志导出功能
- 不实现日志筛选/搜索功能

## Technical Considerations

- 后端 API 重试使用 `tokio::time::sleep` + jitter（±10%）实现延迟
- SSE 重连需处理 task 已完成/不存在的情况（直接返回最终状态并停止重连）
- `TaskData` 使用 `RwLock` 保护，`is_retrying` 用原子或锁内检查防并发
- 滑出抽屉使用 CSS `transform: translateX()` + `transition` 实现动画，不用 `display:none`
- 打开抽屉时设置 `body { overflow:hidden }` 防止背景滚动穿透
- `page_summaries` 按页存储详情，`logs` 限制 50 条系统事件，避免 SSE 体积过大
- 3 个并发任务各 50MB PDF bytes 约 150MB 内存，需监控
- 任务完成/过期后及时清理 `pdf_bytes` 释放内存

## Success Metrics

- 页面更简洁，无历史任务干扰
- 网络波动时自动恢复，减少用户手动刷新
- 单页 API 失败不导致整个任务失败
- 用户可清晰看到每页处理详情

## Open Questions

- 是否需要显示重连倒计时？
- 日志面板是否需要支持复制内容？
