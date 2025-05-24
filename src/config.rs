use std::env;
use anyhow::{Result, Context};

#[derive(Debug, Clone)]
pub struct Config {
    pub discord_token: String,
    pub gemini_api_key: String,
    pub bot_name: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let discord_token = env::var("DISCORD_TOKEN")
            .context("DISCORD_TOKEN environment variable not set")?;
        
        let gemini_api_key = env::var("GEMINI_API_KEY")
            .context("GEMINI_API_KEY environment variable not set")?;
        
        let bot_name = env::var("BOT_NAME").unwrap_or_else(|_| "axis".to_string());
        
        Ok(Config {
            discord_token,
            gemini_api_key,
            bot_name,
        })
    }
}
