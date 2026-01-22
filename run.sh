#!/bin/bash

# 加载 .env 文件（如果存在）
if [ -f .env ]; then
    export $(grep -v '^#' .env | xargs)
fi

# 设置默认值（如果 .env 中未定义）
export OCR_MODEL=${OCR_MODEL:-gemini-3-flash-preview}
export MODEL=${MODEL:-gpt-5.2}
export PORT=${PORT:-8080}

# 检查必需的环境变量
if [ -z "$BASE_URL" ] || [ -z "$API_KEY" ]; then
    echo "错误: 请在 .env 文件中配置 BASE_URL 和 API_KEY"
    echo "参考 .env.example 文件"
    exit 1
fi

echo "启动 PDF 翻译器..."
echo "API: $BASE_URL"
echo "OCR 模型: $OCR_MODEL"
echo "翻译模型: $MODEL"
echo "端口: $PORT"

./target/release/pdftrans
