mod ai;
mod bot;
mod commands;
mod config;

use anyhow::Result;
use bot::{Handler, ShardManagerContainer};
use config::Config;
use serenity::prelude::*;
use std::sync::Arc;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize comprehensive logging
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("axis_bot=debug,serenity=info,tracing=info"));
    
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_thread_ids(true)
        .with_line_number(true)
        .with_file(true)
        .init();
    
    info!("=== Starting Axis Bot ===");
    info!("Rust version: {}", env!("RUSTC_VERSION"));
    info!("Build timestamp: {}", env!("VERGEN_BUILD_TIMESTAMP"));

    // Load configuration
    let config = match Config::from_env() {
        Ok(config) => {
            info!("Configuration loaded successfully");
            info!("Bot name: {}", config.bot_name);
            info!("Discord token length: {}", config.discord_token.len());
            info!("Gemini API key length: {}", config.gemini_api_key.len());
            config
        }
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            error!("Please ensure DISCORD_TOKEN and GEMINI_API_KEY environment variables are set");
            return Err(e);
        }
    };

    // Create handler
    let handler = Handler::new(config.clone());
    info!("Handler created successfully");
    
    // Configure Discord intents
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MEMBERS;

    info!("Configured intents: {:?}", intents);

    // Create Discord client
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
            error!("This usually indicates an invalid Discord token");
            return Err(e.into());
        }
    };

    // Insert shard manager
    {
        let mut data = client.data.write().await;
        data.insert::<ShardManagerContainer>(client.shard_manager.clone());
        info!("Shard manager inserted into client data");
    }

    info!("=== Axis Bot Starting Connection ===");

    // Start the client with proper error handling
    match client.start().await {
        Ok(_) => {
            info!("Client started successfully");
            Ok(())
        },
        Err(e) => {
            error!("Client encountered an error: {:?}", e);
            
            // Provide specific error guidance
            match e {
                serenity::Error::Gateway(gateway_error) => {
                    error!("Gateway error - this usually indicates network or authentication issues");
                    error!("Gateway error details: {:?}", gateway_error);
                },
                serenity::Error::Http(http_error) => {
                    error!("HTTP error - this usually indicates API issues");
                    error!("HTTP error details: {:?}", http_error);
                },
                _ => {
                    error!("Other error type: {:?}", e);
                }
            }
            
            Err(e.into())
        }
    }
}
