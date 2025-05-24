use serenity::builder::{CreateCommand, CreateEmbed, CreateInteractionResponse, CreateInteractionResponseMessage, EditInteractionResponse};
use serenity::model::prelude::*;
use serenity::prelude::*;
use tracing::info;

// src/commands/ping.rs
use serenity::{
    builder::CreateCommand,
    model::prelude::*,
    prelude::*,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::bot::ShardManagerContainer; // ← Your TypeMapKey from bot.rs

/// The actual handler for `/ping`
pub async fn ping(ctx: &Context, command: &CommandInteraction) -> Result<(), serenity::Error> {
    // 1) Pull the ShardManager out of ctx.data
    let data_read = ctx.data.read().await;
    let manager_lock = data_read
        .get::<ShardManagerContainer>()
        .expect("Expected ShardManager in TypeMap")
        .clone();
    drop(data_read);

    // 2) Lock it and measure gateway latency
    let manager = manager_lock.lock().await;
    let gateway_ms = manager
        .shards
        .get(0)
        .and_then(|shard| shard.latency())
        .unwrap_or_default()
        .as_millis();

    // 3) Measure REST latency (defer + edit)
    let rest_start = std::time::Instant::now();
    command.defer(&ctx.http).await?;
    let rest_ms = rest_start.elapsed().as_millis();

    // 4) Edit the deferred response with both numbers
    command
        .edit_response(&ctx.http, |resp| {
            resp.content(format!("Gateway: {}ms\nREST:    {}ms", gateway_ms, rest_ms))
        })
        .await?;

    Ok(())
}

/// How this command shows up to Discord when you register it
pub fn register_ping() -> CreateCommand {
    CreateCommand::new("ping")
        .description("Check the bot's latency")
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

    type ServerInfoData = (String, String, String, UserId, String, String, String, String, String, String, String);
    let guild_info_result: Result<ServerInfoData, ()> = {
        match ctx.cache.guild(guild_id) {
            Some(guild_ref) => {
                let owned_guild = (*guild_ref).clone();
                let created_at = owned_guild.id.created_at();
                let created_date = format!("{}", created_at.date());
                Ok((
                    owned_guild.name.clone(),
                    owned_guild.icon_url().unwrap_or_default(),
                    owned_guild.id.to_string(),
                    owned_guild.owner_id,
                    owned_guild.member_count.to_string(),
                    created_date,
                    owned_guild.roles.len().to_string(),
                    owned_guild.channels.len().to_string(),
                    format!("{:?}", owned_guild.premium_tier),
                    owned_guild.premium_subscription_count.unwrap_or(0).to_string(),
                    format!("{:?}", owned_guild.verification_level),
                ))
            }
            None => Err(()),
        }
    };

    match guild_info_result {
        Ok((
            guild_name,
            icon_url,
            server_id_str,
            owner_id,
            member_count_str,
            created_at_str,
            roles_len_str,
            channels_len_str,
            premium_tier_str,
            boosters_str,
            verification_level_str,
        )) => {
            let owner_tag = owner_id.to_user(&http).await.map_or("Unknown".to_string(), |u| u.tag());
            
            let embed = CreateEmbed::new()
                .title(format!("📊 {}", guild_name))
                .color(0x5865F2)
                .thumbnail(icon_url)
                .field("👑 Owner", owner_tag, true)
                .field("👥 Members", format!("{} members", member_count_str), true)
                .field("📅 Created", created_at_str, true)
                .field("🎭 Roles", roles_len_str, true)
                .field("💬 Channels", channels_len_str, true)
                .field("🚀 Boost Level", premium_tier_str.replace("Tier", "Level"), true)
                .field("💎 Boosters", boosters_str, true)
                .field("🔒 Verification", verification_level_str, true)
                .field("🆔 Server ID", format!("`{}`", server_id_str), false)
                .footer(serenity::builder::CreateEmbedFooter::new("Axis Bot"));
            
            let response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new().embed(embed)
            );
            command.create_response(&http, response).await?;
        }
        Err(_) => {
            let err_response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("Could not fetch server information.")
                    .ephemeral(true)
            );
            command.create_response(&http, err_response).await?;
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

    let guild_data_result: Result<(String, u64), ()> = {
        let guild_option = ctx.cache.guild(guild_id);
        match guild_option {
            Some(guild_ref) => {
                let owned_guild = (*guild_ref).clone();
                Ok((owned_guild.name.clone(), owned_guild.member_count))
            }
            None => Err(()),
        }
    };

    match guild_data_result {
        Ok((guild_name, member_count)) => {
            let embed = CreateEmbed::new()
                .title("👥 Member Statistics")
                .color(0x57F287)
                .field("🏠 Server", guild_name, false)
                .field("📊 Total Members", format!("**{}** members", member_count), false)
                .footer(serenity::builder::CreateEmbedFooter::new("Axis Bot • Member Count"));

            let response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new().embed(embed)
            );
            command.create_response(&http, response).await?;
        }
        Err(_) => {
            let err_response = CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("Could not fetch server information for member count.")
                    .ephemeral(true)
            );
            command.create_response(&http, err_response).await?;
        }
    }

    Ok(())
}

pub fn register_ping() -> CreateCommand {
    CreateCommand::new("ping").description("Check the bot's latency")
}

pub fn register_serverinfo() -> CreateCommand {
    CreateCommand::new("serverinfo").description("Display information about the current server")
}

pub fn register_membercount() -> CreateCommand {
    CreateCommand::new("membercount").description("Display the current member count of the server")
}
