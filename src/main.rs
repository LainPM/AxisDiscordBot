mod ai;
mod bot;
mod commands;
mod config;

use anyhow::Result;
use bot::{Handler, ShardManagerContainer};
use config::Config;
use serenity::prelude::*;
use tracing::{error, info};
use tracing_subscriber;

// Added imports for AI Configuration
use crate::ai::config::{AiConfiguration, AiConfigStore};
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)
        .with_thread_ids(true)
        .init();
    
    info!("Starting Axis bot...");

    let config = match Config::from_env() {
        Ok(config) => {
            info!("Configuration loaded successfully");
            info!("Bot name: {}", config.bot_name);
            config
        }
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return Err(e);
        }
    };

    // Assuming ai_config_arc is defined before this line from the conceptual reordering
    // of main.rs where AiConfiguration is loaded and wrapped before Handler instantiation.
    let handler = Handler::new(config.clone(), ai_config_arc.clone());
    
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILDS;

    info!("Creating Discord client with intents: {:?}", intents);

    let mut client = match Client::builder(&config.discord_token, intents)
        .event_handler(handler)
        .await
    {
        Ok(client) => {
            info!("Discord client created successfully");
            client
        },
        Err(e) => {
            error!("Failed to create Discord client: {}", e);
            return Err(e.into());
        }
    };

    {
        let mut data = client.data.write().await;
        data.insert::<ShardManagerContainer>(client.shard_manager.clone());
        
        // Load and store AI Configuration
        let ai_config = AiConfiguration::load();
        let ai_config_arc = Arc::new(RwLock::new(ai_config));
        data.insert::<AiConfigStore>(ai_config_arc.clone());
        // GeminiClient instantiation will be handled in a later step if needed here.
    }
    info!("AI Configuration initialized and added to client data."); // Added log message

    info!("Axis bot is starting up...");

    if let Err(e) = client.start().await {
        error!("Client error: {:?}", e);
        return Err(e.into());
    }

    Ok(())
}
