use std::env;
use anyhow::{Result, Context};
use tracing::{info, warn, error};

#[derive(Debug, Clone)]
pub struct Config {
    pub discord_token: String,
    pub gemini_api_key: String,
    pub bot_name: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        info!("Loading configuration from environment variables");
        
        // Check for required environment variables
        let discord_token = env::var("DISCORD_TOKEN")
            .context("DISCORD_TOKEN environment variable not set")?;
        
        if discord_token.is_empty() {
            error!("DISCORD_TOKEN is empty");
            return Err(anyhow::anyhow!("DISCORD_TOKEN cannot be empty"));
        }
        
        if discord_token.len() < 50 {
            warn!("DISCORD_TOKEN seems unusually short ({}). This might be incorrect.", discord_token.len());
        }
        
        let gemini_api_key = env::var("GEMINI_API_KEY")
            .context("GEMINI_API_KEY environment variable not set")?;
        
        if gemini_api_key.is_empty() {
            error!("GEMINI_API_KEY is empty");
            return Err(anyhow::anyhow!("GEMINI_API_KEY cannot be empty"));
        }
        
        if gemini_api_key.len() < 30 {
            warn!("GEMINI_API_KEY seems unusually short ({}). This might be incorrect.", gemini_api_key.len());
        }
        
        let bot_name = env::var("BOT_NAME").unwrap_or_else(|_| {
            info!("BOT_NAME not set, using default: 'axis'");
            "axis".to_string()
        });
        
        if bot_name.is_empty() {
            warn!("BOT_NAME is empty, using default: 'axis'");
            return Ok(Config {
                discord_token,
                gemini_api_key,
                bot_name: "axis".to_string(),
            });
        }
        
        info!("Configuration loaded successfully:");
        info!("- Bot name: {}", bot_name);
        info!("- Discord token length: {}", discord_token.len());
        info!("- Gemini API key length: {}", gemini_api_key.len());
        
        Ok(Config {
            discord_token,
            gemini_api_key,
            bot_name,
        })
    }
    
    pub fn validate(&self) -> Result<()> {
        if self.discord_token.is_empty() {
            return Err(anyhow::anyhow!("Discord token is empty"));
        }
        
        if self.gemini_api_key.is_empty() {
            return Err(anyhow::anyhow!("Gemini API key is empty"));
        }
        
        if self.bot_name.is_empty() {
            return Err(anyhow::anyhow!("Bot name is empty"));
        }
        
        Ok(())
    }
}
