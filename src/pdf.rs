use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use lopdf::Document;
use std::process::Command;
use tempfile::TempDir;
use std::fs;

pub struct PdfPage {
    pub page_num: usize,
    pub image_base64: Option<String>,  // None if text extraction succeeded
    pub extracted_text: Option<String>, // Some if text extraction succeeded
}

/// Process PDF pages: always use OCR for reliable text extraction
/// Text extraction from PDF is unreliable due to font encoding issues
pub fn process_pdf_pages(data: &[u8]) -> Result<Vec<PdfPage>, String> {
    let doc = Document::load_mem(data)
        .map_err(|e| format!("Failed to parse PDF: {}", e))?;
    
    let page_count = doc.get_pages().len();
    if page_count == 0 {
        return Err("PDF has no pages".to_string());
    }
    
    // Always use OCR - PDF text extraction is unreliable
    let mut pages: Vec<PdfPage> = Vec::with_capacity(page_count);
    for page_num in 1..=page_count {
        pages.push(PdfPage {
            page_num,
            image_base64: None,
            extracted_text: None,
        });
    }
    
    // Render all pages to images for OCR
    let temp_dir = TempDir::new()
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;
    
    let pdf_path = temp_dir.path().join("input.pdf");
    fs::write(&pdf_path, data)
        .map_err(|e| format!("Failed to write temp PDF: {}", e))?;
    
    let output_prefix = temp_dir.path().join("page");
    let result = Command::new("pdftoppm")
        .args([
            "-jpeg",
            "-jpegopt", "quality=70",
            "-r", "72",
            "-scale-to", "800",
            pdf_path.to_str().unwrap(),
            output_prefix.to_str().unwrap(),
        ])
        .output();
    
    match result {
        Ok(output) if output.status.success() => {
            for page_num in 1..=page_count {
                let image_path = find_page_image(temp_dir.path(), page_num)?;
                let image_data = fs::read(&image_path)
                    .map_err(|e| format!("Failed to read page {} image: {}", page_num, e))?;
                
                pages[page_num - 1].image_base64 = Some(BASE64.encode(&image_data));
            }
            Ok(pages)
        }
        _ => {
            Err("pdftoppm not found. Please install poppler-utils:\n  macOS: brew install poppler\n  Ubuntu: apt install poppler-utils".to_string())
        }
    }
}

/// Extract text from a single page
fn extract_page_text(doc: &Document, page_num: usize) -> String {
    let page_id = match doc.get_pages().get(&(page_num as u32)) {
        Some(id) => *id,
        None => return String::new(),
    };
    
    let content = match doc.get_page_content(page_id) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    
    // Simple text extraction from content stream
    extract_text_from_content(&content, doc)
}

/// Extract readable text from PDF content stream
fn extract_text_from_content(content: &[u8], doc: &Document) -> String {
    let content_str = String::from_utf8_lossy(content);
    let mut text = String::new();
    let mut in_text = false;
    let mut current_text = String::new();
    
    for line in content_str.lines() {
        let line = line.trim();
        
        if line == "BT" {
            in_text = true;
            continue;
        }
        if line == "ET" {
            in_text = false;
            if !current_text.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&current_text);
                current_text.clear();
            }
            continue;
        }
        
        if in_text {
            // Handle text operators: Tj, TJ, ', "
            if let Some(extracted) = extract_text_operator(line, doc) {
                current_text.push_str(&extracted);
            }
        }
    }
    
    text
}

/// Extract text from PDF text operators
fn extract_text_operator(line: &str, _doc: &Document) -> Option<String> {
    let line = line.trim();
    
    // Handle (text) Tj
    if line.ends_with(" Tj") || line.ends_with(")Tj") {
        if let Some(start) = line.find('(') {
            if let Some(end) = line.rfind(')') {
                let text = &line[start + 1..end];
                return Some(decode_pdf_string(text));
            }
        }
    }
    
    // Handle <hex> Tj
    if line.ends_with(" Tj") || line.ends_with(">Tj") {
        if let Some(start) = line.find('<') {
            if let Some(end) = line.rfind('>') {
                let hex = &line[start + 1..end];
                return decode_hex_string(hex);
            }
        }
    }
    
    // Handle [ ... ] TJ (array of strings)
    if line.ends_with(" TJ") || line.ends_with("]TJ") {
        let mut result = String::new();
        let mut i = 0;
        let chars: Vec<char> = line.chars().collect();
        
        while i < chars.len() {
            if chars[i] == '(' {
                let start = i + 1;
                i += 1;
                while i < chars.len() && chars[i] != ')' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i > start {
                    let text: String = chars[start..i].iter().collect();
                    result.push_str(&decode_pdf_string(&text));
                }
            } else if chars[i] == '<' {
                let start = i + 1;
                i += 1;
                while i < chars.len() && chars[i] != '>' {
                    i += 1;
                }
                if i > start {
                    let hex: String = chars[start..i].iter().collect();
                    if let Some(decoded) = decode_hex_string(&hex) {
                        result.push_str(&decoded);
                    }
                }
            }
            i += 1;
        }
        
        if !result.is_empty() {
            return Some(result);
        }
    }
    
    None
}

/// Decode PDF string escapes
fn decode_pdf_string(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('(') => result.push('('),
                Some(')') => result.push(')'),
                Some(c) => result.push(c),
                None => {}
            }
        } else {
            result.push(c);
        }
    }
    
    result
}

/// Decode hex string to text
fn decode_hex_string(hex: &str) -> Option<String> {
    let hex = hex.replace(" ", "");
    if hex.len() % 4 == 0 {
        // Try UTF-16BE (common for CJK)
        let mut chars = Vec::new();
        for i in (0..hex.len()).step_by(4) {
            if let Ok(code) = u16::from_str_radix(&hex[i..i+4], 16) {
                if let Some(c) = char::from_u32(code as u32) {
                    chars.push(c);
                }
            }
        }
        if !chars.is_empty() {
            return Some(chars.into_iter().collect());
        }
    }
    
    // Try simple hex decoding
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&hex[i..i.min(hex.len()).max(i+2)], 16).ok())
        .collect();
    
    String::from_utf8(bytes).ok()
}

/// Check if extracted text is valid (not empty, not garbled)
fn is_text_valid(text: &str) -> bool {
    let text = text.trim();
    
    // Too short - probably failed extraction
    if text.len() < 50 {
        return false;
    }
    
    // Count readable characters (must be actual readable text, not PDF encoding artifacts)
    let total_chars = text.chars().count();
    
    // Check for common PDF encoding issues
    // Many PDFs use CID fonts which produce garbage when decoded naively
    let control_chars = text.chars().filter(|c| {
        let code = *c as u32;
        // Control characters, private use area, or replacement char
        code < 32 || (0xE000..=0xF8FF).contains(&code) || code == 0xFFFD
    }).count();
    
    // If more than 5% control/private chars, it's garbage
    if control_chars as f32 / total_chars as f32 > 0.05 {
        return false;
    }
    
    // Count actual readable characters
    let readable_chars = text.chars().filter(|c| {
        c.is_ascii_alphanumeric() || 
        c.is_ascii_whitespace() || 
        c.is_ascii_punctuation() ||
        // CJK ranges
        ('\u{4E00}'..='\u{9FFF}').contains(c) ||
        ('\u{3040}'..='\u{30FF}').contains(c) ||  // Japanese
        ('\u{AC00}'..='\u{D7AF}').contains(c) ||  // Korean
        // Common CJK punctuation
        "，。！？、；：（）【】《》「」『』".contains(*c)
    }).count();
    
    // At least 80% should be readable (stricter threshold)
    let ratio = readable_chars as f32 / total_chars as f32;
    
    // Also require minimum word-like structure (has spaces or CJK)
    let has_structure = text.contains(' ') || 
        text.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
    
    ratio > 0.8 && has_structure
}

fn find_page_image(dir: &std::path::Path, page_num: usize) -> Result<std::path::PathBuf, String> {
    // Try different naming patterns (JPEG format)
    let patterns = [
        format!("page-{}.jpg", page_num),
        format!("page-{:02}.jpg", page_num),
        format!("page-{:03}.jpg", page_num),
    ];
    
    for pattern in &patterns {
        let path = dir.join(pattern);
        if path.exists() {
            return Ok(path);
        }
    }
    
    Err(format!("Image for page {} not found", page_num))
}

pub fn generate_pdf(pages: &[String]) -> Result<Vec<u8>, String> {
    let mut pdf = SimplePdf::new();
    
    for page_content in pages {
        pdf.add_content(page_content);
    }
    
    pdf.render()
}

struct SimplePdf {
    content: String,
}

impl SimplePdf {
    fn new() -> Self {
        Self { content: String::new() }
    }
    
    fn add_content(&mut self, text: &str) {
        if !self.content.is_empty() {
            self.content.push_str("\n\n");
        }
        self.content.push_str(text);
    }
    
    fn render(&self) -> Result<Vec<u8>, String> {
        let mut output: Vec<u8> = Vec::new();
        output.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
        
        let mut obj_offsets: Vec<usize> = Vec::new();
        let page_contents = self.prepare_pages();
        let num_pages = page_contents.len();
        
        obj_offsets.push(output.len());
        output.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        
        obj_offsets.push(output.len());
        let page_refs: String = (0..num_pages)
            .map(|i| format!("{} 0 R", 4 + i * 2))
            .collect::<Vec<_>>()
            .join(" ");
        let pages_obj = format!(
            "2 0 obj\n<< /Type /Pages /Kids [ {} ] /Count {} >>\nendobj\n",
            page_refs, num_pages
        );
        output.extend_from_slice(pages_obj.as_bytes());
        
        // CJK Font
        obj_offsets.push(output.len());
        output.extend_from_slice(
            b"3 0 obj\n<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light \
              /Encoding /UniGB-UTF16-H \
              /DescendantFonts [ << /Type /Font /Subtype /CIDFontType0 \
              /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) \
              /Ordering (GB1) /Supplement 5 >> >> ] >>\nendobj\n"
        );
        
        for (i, content_stream) in page_contents.iter().enumerate() {
            let page_obj_num = 4 + i * 2;
            let content_obj_num = 5 + i * 2;
            
            obj_offsets.push(output.len());
            let page_obj = format!(
                "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
                 /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>\nendobj\n",
                page_obj_num, content_obj_num
            );
            output.extend_from_slice(page_obj.as_bytes());
            
            obj_offsets.push(output.len());
            let content_obj = format!(
                "{} 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
                content_obj_num, content_stream.len(), content_stream
            );
            output.extend_from_slice(content_obj.as_bytes());
        }
        
        let xref_offset = output.len();
        let xref_header = format!("xref\n0 {}\n", obj_offsets.len() + 1);
        output.extend_from_slice(xref_header.as_bytes());
        output.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &obj_offsets {
            let line = format!("{:010} 00000 n \n", offset);
            output.extend_from_slice(line.as_bytes());
        }
        
        let trailer = format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            obj_offsets.len() + 1,
            xref_offset
        );
        output.extend_from_slice(trailer.as_bytes());
        
        Ok(output)
    }
    
    fn prepare_pages(&self) -> Vec<String> {
        let font_size = 11.0;
        let line_height = 16.0;
        let margin_left = 50.0;
        let margin_top = 50.0;
        let margin_bottom = 50.0;
        let page_height = 842.0;
        let page_width = 595.0;
        let usable_height = page_height - margin_top - margin_bottom;
        let usable_width = page_width - margin_left * 2.0;
        
        let char_width = font_size * 0.55;
        let max_chars = (usable_width / char_width) as usize;
        let max_lines_per_page = (usable_height / line_height) as usize;
        
        let mut pages: Vec<String> = Vec::new();
        let mut current_page_lines: Vec<String> = Vec::new();
        
        for line in self.content.lines() {
            let wrapped = self.wrap_text(line, max_chars);
            for wrapped_line in wrapped {
                if current_page_lines.len() >= max_lines_per_page {
                    pages.push(self.create_page_stream(&current_page_lines, font_size, line_height, margin_left, page_height - margin_top));
                    current_page_lines.clear();
                }
                current_page_lines.push(wrapped_line);
            }
        }
        
        if !current_page_lines.is_empty() || pages.is_empty() {
            pages.push(self.create_page_stream(&current_page_lines, font_size, line_height, margin_left, page_height - margin_top));
        }
        
        pages
    }
    
    fn create_page_stream(&self, lines: &[String], font_size: f64, line_height: f64, margin_left: f64, start_y: f64) -> String {
        let mut stream = String::new();
        stream.push_str("BT\n");
        stream.push_str(&format!("/F1 {} Tf\n", font_size));
        stream.push_str(&format!("{} TL\n", line_height));
        stream.push_str(&format!("1 0 0 1 {} {} Tm\n", margin_left, start_y));
        
        for line in lines {
            if line.is_empty() {
                stream.push_str("T*\n");
            } else {
                stream.push_str(&format!("<{}> Tj T*\n", self.to_utf16be_hex(line)));
            }
        }
        
        stream.push_str("ET\n");
        stream
    }
    
    fn wrap_text(&self, text: &str, max_chars: usize) -> Vec<String> {
        if text.is_empty() {
            return vec![String::new()];
        }
        
        let mut lines = Vec::new();
        let mut current = String::new();
        let mut count = 0;
        
        for c in text.chars() {
            let char_width = if c.is_ascii() { 1 } else { 2 };
            if count + char_width > max_chars && !current.is_empty() {
                lines.push(current);
                current = String::new();
                count = 0;
            }
            current.push(c);
            count += char_width;
        }
        
        if !current.is_empty() {
            lines.push(current);
        }
        
        if lines.is_empty() {
            lines.push(String::new());
        }
        
        lines
    }
    
    fn to_utf16be_hex(&self, text: &str) -> String {
        let mut hex = String::with_capacity(text.len() * 4 + 4);
        hex.push_str("FEFF");
        
        for c in text.chars() {
            let code = c as u32;
            if code <= 0xFFFF {
                hex.push_str(&format!("{:04X}", code));
            } else {
                let adjusted = code - 0x10000;
                let high = 0xD800 + ((adjusted >> 10) & 0x3FF);
                let low = 0xDC00 + (adjusted & 0x3FF);
                hex.push_str(&format!("{:04X}{:04X}", high, low));
            }
        }
        hex
    }
}
