#[derive(Clone)]
pub struct Config {
    pub base_url: String,
    pub api_key: String,
    pub ocr_model: String,
    pub translate_model: String,
    pub ocr_model_fallback: Option<String>,
    pub translate_model_fallback: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("BASE_URL")
                .expect("BASE_URL environment variable is required"),
            api_key: std::env::var("API_KEY")
                .expect("API_KEY environment variable is required"),
            ocr_model: std::env::var("OCR_MODEL")
                .unwrap_or_else(|_| "gemini-3-flash-preview".to_string()),
            translate_model: std::env::var("MODEL")
                .unwrap_or_else(|_| "gpt-5.2".to_string()),
            ocr_model_fallback: std::env::var("OCR_MODEL_FALLBACK").ok().filter(|s| !s.is_empty()),
            translate_model_fallback: std::env::var("MODEL_FALLBACK").ok().filter(|s| !s.is_empty()),
        }
    }
}
