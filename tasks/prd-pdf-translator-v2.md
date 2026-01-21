# PRD: PDF 多语言翻译器 V2

## Introduction

使用 AI 视觉模型识别 PDF 内容，解决传统文本提取的编码兼容性问题。支持任意语言的 PDF（包括日文、韩文、阿拉伯文等）翻译成中文。

**核心流程：**
```
PDF → 转图片 → Gemini 识别文本 → GPT-5.2 翻译 → 生成 PDF
```

## Goals

- 支持任意语言 PDF 的文本识别（通过视觉模型）
- 使用 GPT-5.2 进行高质量翻译
- 保持原文格式（标题、段落、列表）
- 实时显示处理进度
- 轻量级部署，适合低端 VPS

## User Stories

### US-001: 配置多模型支持
**Description:** 作为开发者，我需要配置两个模型：OCR 识别模型和翻译模型。

**Acceptance Criteria:**
- [ ] 新增环境变量 `OCR_MODEL`，默认 `gemini-3-flash-preview`
- [ ] 保留 `MODEL` 环境变量用于翻译，默认 `gpt-5.2`
- [ ] 两个模型共用同一个 API 端点和密钥
- [ ] `cargo build` 成功

### US-002: PDF 转图片
**Description:** 作为系统，我需要将 PDF 每一页转换为图片以便视觉模型识别。

**Acceptance Criteria:**
- [ ] 使用 `pdfium-render` 或 `pdf2image` 将 PDF 页面渲染为 PNG
- [ ] 图片分辨率适中（150 DPI），平衡质量和大小
- [ ] 返回每页图片的 base64 编码
- [ ] 处理失败时返回明确错误

### US-003: 视觉模型文本识别
**Description:** 作为系统，我需要调用 Gemini 模型识别图片中的文本。

**Acceptance Criteria:**
- [ ] 构建包含图片的 multimodal 请求
- [ ] Prompt: "请识别图片中的所有文本内容，保持原文格式和换行"
- [ ] 正确解析模型返回的文本
- [ ] 支持多语言识别（日文、韩文、英文等）

### US-004: 文本翻译
**Description:** 作为系统，我需要使用 GPT-5.2 将识别的文本翻译成中文。

**Acceptance Criteria:**
- [ ] 使用配置的翻译模型（默认 gpt-5.2）
- [ ] Prompt 强调保持格式
- [ ] 如果内容已经是中文，跳过翻译
- [ ] 处理长文本分段

### US-005: 进度显示优化
**Description:** 作为用户，我想看到详细的处理进度。

**Acceptance Criteria:**
- [ ] 显示当前步骤：识别中/翻译中
- [ ] 显示页码进度：第 X 页 / 共 Y 页
- [ ] 识别和翻译分开显示进度

### US-006: 生成翻译 PDF
**Description:** 作为用户，我想下载翻译后的 PDF 文件。

**Acceptance Criteria:**
- [ ] 生成包含中文内容的 PDF
- [ ] 保持基本格式（段落、换行）
- [ ] PDF 文件大小合理（< 1MB）

## Functional Requirements

- FR-1: 环境变量配置
  - `BASE_URL`: API 端点（必需）
  - `API_KEY`: API 密钥（必需）
  - `OCR_MODEL`: 识别模型（默认 gemini-3-flash-preview）
  - `MODEL`: 翻译模型（默认 gpt-5.2）
  - `PORT`: 服务端口（默认 8080）

- FR-2: PDF 处理流程
  1. 接收上传的 PDF 文件
  2. 将每页渲染为 PNG 图片
  3. 调用 OCR 模型识别每页文本
  4. 调用翻译模型翻译文本
  5. 生成翻译后的 PDF

- FR-3: API 请求格式（视觉模型）
  ```json
  {
    "model": "gemini-3-flash-preview",
    "messages": [{
      "role": "user",
      "content": [
        {"type": "text", "text": "识别图片中的文本..."},
        {"type": "image_url", "image_url": {"url": "data:image/png;base64,..."}}
      ]
    }]
  }
  ```

- FR-4: 进度状态
  - `Rendering`: 正在渲染 PDF
  - `Recognizing`: 正在识别第 X 页
  - `Translating`: 正在翻译第 X 页
  - `Generating`: 正在生成 PDF
  - `Complete`: 完成
  - `Error`: 错误信息

## Non-Goals

- 不做复杂排版保留（表格、多栏、图片位置）
- 不做 OCR 结果的人工校对功能
- 不做批量文件处理
- 不做翻译记忆/术语库

## Technical Considerations

- **PDF 渲染**: 使用 `pdfium-render` crate（需要 pdfium 库）或外部工具
- **图片格式**: PNG，150 DPI，base64 编码
- **模型调用**: 复用现有 HTTP 客户端，支持 multimodal content
- **内存管理**: 逐页处理，避免一次性加载所有图片

## Success Metrics

- 成功处理包含日文编码的 PDF
- 翻译准确率 > 90%
- 单页处理时间 < 30 秒
- 支持 20 页以内的 PDF

## Open Questions

- 是否需要支持 PDF 中的表格识别？（当前方案：不支持）
