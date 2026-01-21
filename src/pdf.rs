use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use lopdf::Document;
use std::process::Command;
use tempfile::TempDir;
use std::fs;

pub struct PdfPage {
    pub page_num: usize,
    pub image_base64: String,
}

/// Render PDF pages to images using pdftoppm (from poppler-utils)
/// Falls back to extracting text if pdftoppm is not available
pub fn render_pdf_pages(data: &[u8]) -> Result<Vec<PdfPage>, String> {
    // Create temp directory
    let temp_dir = TempDir::new()
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;
    
    let pdf_path = temp_dir.path().join("input.pdf");
    fs::write(&pdf_path, data)
        .map_err(|e| format!("Failed to write temp PDF: {}", e))?;
    
    // Get page count from lopdf
    let doc = Document::load_mem(data)
        .map_err(|e| format!("Failed to parse PDF: {}", e))?;
    let page_count = doc.get_pages().len();
    
    if page_count == 0 {
        return Err("PDF has no pages".to_string());
    }
    
    // Try to use pdftoppm with lower DPI for smaller images
    let output_prefix = temp_dir.path().join("page");
    let result = Command::new("pdftoppm")
        .args([
            "-png",
            "-r", "100",  // 100 DPI - balance quality and API limits
            "-scale-to", "1200",  // Max dimension
            pdf_path.to_str().unwrap(),
            output_prefix.to_str().unwrap(),
        ])
        .output();
    
    match result {
        Ok(output) if output.status.success() => {
            // Read generated images
            let mut pages = Vec::with_capacity(page_count);
            
            for i in 1..=page_count {
                // pdftoppm generates files like page-1.png, page-01.png, or page-001.png
                let image_path = find_page_image(temp_dir.path(), i)?;
                
                let image_data = fs::read(&image_path)
                    .map_err(|e| format!("Failed to read page {} image: {}", i, e))?;
                
                pages.push(PdfPage {
                    page_num: i,
                    image_base64: BASE64.encode(&image_data),
                });
            }
            
            Ok(pages)
        }
        _ => {
            // pdftoppm not available, try alternative approach
            Err("pdftoppm not found. Please install poppler-utils:\n  macOS: brew install poppler\n  Ubuntu: apt install poppler-utils".to_string())
        }
    }
}

fn find_page_image(dir: &std::path::Path, page_num: usize) -> Result<std::path::PathBuf, String> {
    // Try different naming patterns
    let patterns = [
        format!("page-{}.png", page_num),
        format!("page-{:02}.png", page_num),
        format!("page-{:03}.png", page_num),
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
