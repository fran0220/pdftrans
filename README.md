# PDF 多语言翻译器 V2

使用 AI 视觉模型识别 + 翻译模型，将任意语言的 PDF 翻译成中文。

## 特性

- **多语言支持**: 英文、日文、韩文、阿拉伯文等
- **视觉识别**: 使用 Gemini 模型识别 PDF 图像中的文本
- **高质量翻译**: 使用 GPT-5.2 进行翻译
- **格式保留**: 保持原文的标题、段落、列表结构
- **实时进度**: SSE 实时显示识别/翻译进度
- **轻量部署**: 适合低端 VPS

## 依赖

- **poppler-utils**: 用于 PDF 转图片
  - macOS: `brew install poppler`
  - Ubuntu: `apt install poppler-utils`

## 构建

```bash
cargo build --release
```

## 配置

| 环境变量 | 必需 | 默认值 | 说明 |
|---------|------|--------|------|
| BASE_URL | ✅ | - | API 端点 |
| API_KEY | ✅ | - | API 密钥 |
| OCR_MODEL | ❌ | gemini-3-flash-preview | 视觉识别模型 |
| MODEL | ❌ | gpt-5.2 | 翻译模型 |
| PORT | ❌ | 8080 | 服务端口 |

## 运行

```bash
export BASE_URL=http://your-api-endpoint
export API_KEY=your-api-key
./target/release/pdftrans
```

或使用启动脚本：
```bash
./run.sh
```

访问 http://localhost:8080

## 处理流程

```
PDF 上传 → 渲染为图片 → Gemini 识别文本 → GPT-5.2 翻译 → 生成 PDF
```

## API

| 路由 | 方法 | 说明 |
|------|------|------|
| `/` | GET | 主页 |
| `/upload` | POST | 上传 PDF (multipart/form-data) |
| `/progress/{task_id}` | GET | SSE 进度流 |
| `/download/{task_id}` | GET | 下载翻译后的 PDF |

## 进度状态

- `Rendering`: 渲染 PDF 为图片
- `Recognizing`: 识别第 X 页文本
- `Translating`: 翻译第 X 页
- `Generating`: 生成 PDF
- `Complete`: 完成
- `Error`: 错误

## 限制

- 最大文件: 50MB
- 推荐页数: ≤20 页
- 单页处理时间: ~10-30 秒

## 部署

### systemd

```bash
cat > /etc/systemd/system/pdftrans.service << EOF
[Unit]
Description=PDF Translator V2
After=network.target

[Service]
Type=simple
Environment=BASE_URL=http://your-api-endpoint
Environment=API_KEY=your-api-key
Environment=OCR_MODEL=gemini-3-flash-preview
Environment=MODEL=gpt-5.2
Environment=PORT=8080
ExecStart=/opt/pdftrans/pdftrans
Restart=always

[Install]
WantedBy=multi-user.target
EOF

systemctl enable pdftrans
systemctl start pdftrans
```

### Docker

```dockerfile
FROM ubuntu:22.04
RUN apt update && apt install -y poppler-utils ca-certificates
COPY target/release/pdftrans /app/pdftrans
WORKDIR /app
ENV PORT=8080
EXPOSE 8080
CMD ["/app/pdftrans"]
```
