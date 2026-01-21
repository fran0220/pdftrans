#!/bin/bash
export BASE_URL=http://67.230.171.248:8317
export API_KEY=sk-123456
export OCR_MODEL=gemini-3-flash-preview
export MODEL=gpt-5.2
export PORT=8080
./target/release/pdftrans
