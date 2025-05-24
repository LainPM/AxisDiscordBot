use serenity::builder::{CreateCommand, CreateEmbed, CreateInteractionResponse, CreateInteractionResponseMessage, EditInteractionResponse};
use serenity::model::prelude::*;
use serenity::prelude::*;
use chrono::{DateTime, Utc};
use tracing::{info, error, debug};
use std::time::Instant;

pub async fn ping(ctx: &Context, command: &CommandInteraction) -> Result<(), serenity::Error> {
    info!("Ping command executed by {}", command.user.tag());
    let http = ctx.http.clone();
    
    // Initial response to avoid timeout
    let initial_response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content("Calculating ping...")
            .ephemeral(false)
    );
    command.create_response(&http, initial_response).await?;
    
    let start = Instant::now();
    
    // Make a test API call to measure latency
    let _test_call = command.get_response(&http).await;
    let api_latency = start.elapsed().as_millis();
    
    // Get WebSocket latency
    let ws_latency = {
        let shard_manager = ctx.shard_manager.lock().await;
        let shard_runners = shard_manager.runners.lock().await;
        
        if let Some((_, info)) = shard_runners.iter().next() {
            info.latency.map(|d| d.as_millis()).unwrap_or(0)
        } else {
            0
        }
    };
    
    debug!("Ping results - API: {}ms, WebSocket: {}ms", api_latency, ws_latency);
    
    let embed = CreateEmbed::new()
        .title("Connection Status")
        .color(0x00FF00)
        .field("API Latency", format!("{}ms", api_latency), true)
        .field("WebSocket Latency", format!("{}ms", ws_latency), true)
        .field("Status", if api_latency < 100 { "Excellent" } else if api_latency < 300 { "Good" } else { "High" }, true)
        .timestamp(Utc::now())
        .footer(serenity::builder::CreateEmbedFooter::new("Axis Bot"));
    
    let edit_response = EditInteractionResponse::new()
        .content("")
        .embed(embed);
        
    command.edit_response(&http, edit_response).await?;
    
    Ok(())
}

pub async fn serverinfo(ctx: &Context, command: &CommandInteraction) -> Result<(), serenity::Error> {
    let http = ctx.http.clone();
    let guild_id = match command.guild_id {
        Some(id) => id,
        None => {
            let response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("This command can only be used in a server.")
                    .ephemeral(true)
            );
            command.create_response(&http, response).await?;
            return Ok(());
        }
    };

    info!("Serverinfo command executed by {} in guild {}", command.user.tag(), guild_id);

    // Defer the response to avoid timeout
    command.defer(&http).await?;

    let guild_data = match ctx.cache.guild(guild_id) {
        Some(guild_ref) => {
            let guild = guild_ref.clone();
            let created_at_unix = guild.id.created_at().unix_timestamp();
            let created_at: DateTime<Utc> = DateTime::from_timestamp(created_at_unix, 0)
                .expect("Invalid timestamp from Discord API");
                
            let owner_tag = match guild.owner_id.to_user(&http).await {
                Ok(user) => user.tag(),
                Err(e) => {
                    error!("Failed to fetch owner info: {}", e);
                    "Unknown".to_string()
                }
            };
            
            Some((
                guild.name.clone(),
                guild.icon_url().unwrap_or_default(),
                guild.id.to_string(),
                owner_tag,
                guild.member_count,
                created_at.format("%B %d, %Y").to_string(),
                guild.roles.len(),
                guild.channels.len(),
                guild.premium_tier,
                guild.premium_subscription_count.unwrap_or(0),
                guild.verification_level,
            ))
        }
        None => {
            error!("Guild not found in cache: {}", guild_id);
            None
        }
    };

    match guild_data {
        Some((
            guild_name,
            icon_url,
            server_id_str,
            owner_tag,
            member_count,
            created_at_str,
            roles_len,
            channels_len,
            premium_tier,
            boosters,
            verification_level,
        )) => {
            let embed = CreateEmbed::new()
                .title(format!("Server Information: {}", guild_name))
                .color(0x5865F2)
                .thumbnail(icon_url)
                .field("Owner", owner_tag, true)
                .field("Members", format!("{}", member_count), true)
                .field("Created", created_at_str, true)
                .field("Roles", roles_len.to_string(), true)
                .field("Channels", channels_len.to_string(), true)
                .field("Boost Level", format!("Level {}", premium_tier as u8), true)
                .field("Boosters", boosters.to_string(), true)
                .field("Verification Level", format!("{:?}", verification_level), true)
                .field("Server ID", format!("`{}`", server_id_str), false)
                .footer(serenity::builder::CreateEmbedFooter::new("Axis Bot"))
                .timestamp(Utc::now());
            
            let edit_response = EditInteractionResponse::new().embed(embed);
            command.edit_response(&http, edit_response).await?;
        }
        None => {
            let edit_response = EditInteractionResponse::new()
                .content("Could not retrieve server information.");
            command.edit_response(&http, edit_response).await?;
        }
    }

    Ok(())
}

pub async fn membercount(ctx: &Context, command: &CommandInteraction) -> Result<(), serenity::Error> {
    let http = ctx.http.clone();
    let guild_id = match command.guild_id {
        Some(id) => id,
        None => {
            let response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("This command can only be used in a server.")
                    .ephemeral(true)
            );
            command.create_response(&http, response).await?;
            return Ok(());
        }
    };

    info!("Membercount command executed by {} in guild {}", command.user.tag(), guild_id);

    let guild_data = match ctx.cache.guild(guild_id) {
        Some(guild_ref) => {
            let guild = guild_ref.clone();
            Some((guild.name.clone(), guild.member_count))
        }
        None => {
            error!("Guild not found in cache: {}", guild_id);
            None
        }
    };

    match guild_data {
        Some((guild_name, member_count)) => {
            let embed = CreateEmbed::new()
                .title("Member Count")
                .color(0x57F287)
                .field("Server", guild_name, false)
                .field("Total Members", format!("{} members", member_count), false)
                .footer(serenity::builder::CreateEmbedFooter::new("Axis Bot"))
                .timestamp(Utc::now());

            let response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new().embed(embed)
            );
            command.create_response(&http, response).await?;
        }
        None => {
            let err_response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("Could not retrieve server information.")
                    .ephemeral(true)
            );
            command.create_response(&http, err_response).await?;
        }
    }

    Ok(())
}

pub fn register_ping() -> CreateCommand {
    CreateCommand::new("ping")
        .description("Check the bot's connection latency and status")
}

pub fn register_serverinfo() -> CreateCommand {
    CreateCommand::new("serverinfo")
        .description("Display detailed information about the current server")
}

pub fn register_membercount() -> CreateCommand {
    CreateCommand::new("membercount")
        .description("Display the current member count of the server")
}
