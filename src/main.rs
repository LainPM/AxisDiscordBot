mod ai;
mod bot;
mod commands;
mod config;

use anyhow::Result;
use bot::{Handler, ShardManagerContainer}; // Assuming Handler is from bot
use config::Config; // General bot config
use serenity::prelude::*;
use tracing::{error, info};
use tracing_subscriber;

// Imports for AI Configuration
use crate::ai::config::{AiConfiguration, AiConfigStore}; // Path to AI config structs
use std::sync::Arc;
use tokio::sync::RwLock; // Or std::sync::RwLock if AiConfiguration uses that

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)
        .with_thread_ids(true)
        .init();
    
    info!("Starting Axis bot...");

    // 1. Load general bot configuration
    let config = match Config::from_env() {
        Ok(cfg) => {
            info!("General configuration loaded successfully");
            info!("Bot name: {}", cfg.bot_name);
            cfg
        }
        Err(e) => {
            error!("Failed to load general configuration: {}", e);
            return Err(e); // Or e.into() depending on Result type
        }
    };

    // 2. Load AI Configuration & Create Arc
    let ai_config = crate::ai::config::AiConfiguration::load();
    let ai_config_arc = Arc::new(RwLock::new(ai_config)); // ai_config_arc is defined here
    info!("AI Configuration loaded.");

    // 3. Create Handler, passing the ai_config_arc
    let handler = Handler::new(config.clone(), ai_config_arc.clone()); // ai_config_arc is used here
    info!("Event Handler created.");

    // 4. Build Serenity Client, passing the handler
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILDS;
    info!("Creating Discord client with intents: {:?}", intents);

    let mut client = match Client::builder(&config.discord_token, intents)
        .event_handler(handler) // Use the handler created *with* ai_config_arc
        .await
    {
        Ok(c) => {
            info!("Discord client created successfully");
            c
        },
        Err(e) => {
            error!("Failed to create Discord client: {}", e);
            return Err(e.into()); // Or anyhow::Error(e)
        }
    };

    // 5. Insert AiConfigStore and ShardManagerContainer into client.data
    {
        let mut data = client.data.write().await;
        data.insert::<AiConfigStore>(ai_config_arc.clone()); // Insert the same Arc
        data.insert::<ShardManagerContainer>(client.shard_manager.clone());
    }
    info!("AI Configuration and ShardManagerContainer added to client data.");

    info!("Axis bot is starting up...");

    if let Err(e) = client.start().await {
        error!("Client error: {:?}", e);
        return Err(e.into());
    }

    Ok(())
}
