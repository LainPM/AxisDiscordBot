use serenity::all::{CommandOptionType, ResolvedOption, ResolvedValue};
use serenity::builder::{CreateCommand, CreateCommandOption, CreateInteractionResponse, CreateInteractionResponseMessage};
use serenity::model::application::CommandInteraction;
use serenity::model::id::{ChannelId, GuildId};
use serenity::model::permissions::Permissions;
use serenity::prelude::*;
use tracing::{debug, error, info};

use crate::ai::config::{AiConfigStore, AiGuildConfig, AiMode}; // Assuming ai::config is now available

pub async fn run(ctx: &Context, command: &CommandInteraction) -> Result<(), serenity::Error> {
    command.defer_ephemeral(&ctx.http).await?;

    // 1. Check for administrator permissions
    let member_permissions = command.member.as_ref().map(|m| m.permissions).flatten();
    if !member_permissions.map_or(false, |p| p.contains(Permissions::ADMINISTRATOR)) {
        let response_msg = CreateInteractionResponseMessage::new()
            .content("You must be an administrator to use this command.")
            .ephemeral(true);
        command.create_followup(&ctx.http, response_msg).await?;
        return Ok(());
    }

    let guild_id = match command.guild_id {
        Some(id) => id,
        None => {
            let response_msg = CreateInteractionResponseMessage::new()
                .content("This command can only be used in a server.")
                .ephemeral(true);
            command.create_followup(&ctx.http, response_msg).await?;
            return Ok(());
        }
    };

    let mut mode_opt: Option<AiMode> = None;
    let mut targets_opt: Option<Vec<String>> = None;

    // 2. Parse command options
    for option in &command.data.options {
        match option.name {
            "mode" => {
                if let ResolvedValue::String(mode_str) = option.value {
                    mode_opt = match mode_str {
                        "off" => Some(AiMode::Off),
                        "global" => Some(AiMode::Global),
                        "specific" => Some(AiMode::Specific),
                        _ => None, // Should not happen due to choices
                    };
                }
            }
            "targets" => {
                if let ResolvedValue::String(targets_str) = option.value {
                    let parsed_targets: Vec<String> = targets_str
                        .split_whitespace()
                        .filter_map(|s| {
                            // Try to parse as <#ID> or just ID
                            if s.starts_with("<#") && s.ends_with('>') {
                                Some(s[2..s.len()-1].to_string())
                            } else if s.parse::<u64>().is_ok() { // Basic check if it's a plain ID
                                Some(s.to_string())
                            } else {
                                // Could add more sophisticated parsing/validation here if needed
                                // For now, just take strings that look like IDs or are mentions
                                None 
                            }
                        })
                        .collect();
                    if !parsed_targets.is_empty() {
                        targets_opt = Some(parsed_targets);
                    }
                }
            }
            _ => {}
        }
    }

    let mode = match mode_opt {
        Some(m) => m,
        None => {
            let response_msg = CreateInteractionResponseMessage::new()
                .content("Invalid mode selected.")
                .ephemeral(true);
            command.create_followup(&ctx.http, response_msg).await?;
            return Ok(());
        }
    };

    if mode == AiMode::Specific && targets_opt.is_none() {
        let response_msg = CreateInteractionResponseMessage::new()
            .content("For 'specific' mode, you must provide target channels/categories.")
            .ephemeral(true);
        command.create_followup(&ctx.http, response_msg).await?;
        return Ok(());
    }

    // 3. Retrieve AiConfiguration
    let data_read = ctx.data.read().await;
    let config_store_lock = match data_read.get::<AiConfigStore>() {
        Some(store) => store.clone(),
        None => {
            error!("AiConfigStore not found in TypeMap. Cannot configure AI.");
            let response_msg = CreateInteractionResponseMessage::new()
                .content("AI Configuration system is not available. Please contact bot support.")
                .ephemeral(true);
            command.create_followup(&ctx.http, response_msg).await?;
            return Ok(());
        }
    };
    drop(data_read); // Release read lock on TypeMap

    // 4. Update configuration
    { // Scope for RwLockWriteGuard
        let mut config_w = config_store_lock.write().await;
        let guild_config = AiGuildConfig {
            mode,
            allowed_ids: if mode == AiMode::Specific { targets_opt.clone().unwrap_or_default() } else { Vec::new() }, // .clone() targets_opt
        };
        config_w.set_guild_config(guild_id, guild_config.clone());
        
        if let Err(e) = config_w.save() {
            error!("Failed to save AI configuration for guild {}: {}", guild_id, e);
            let response_msg = CreateInteractionResponseMessage::new()
                .content(format!("Error saving AI configuration: {}", e))
                .ephemeral(true);
            command.create_followup(&ctx.http, response_msg).await?;
            return Ok(());
        }
        info!("AI configuration updated for guild {}: Mode={:?}, Targets={:?}", guild_id, guild_config.mode, guild_config.allowed_ids);
    }


    // 5. Send confirmation
    let targets_display = targets_opt.as_ref().map_or_else(
        || "N/A".to_string(),
        |ids| ids.iter().map(|id| format!("<#{}>", id)).collect::<Vec<_>>().join(" ")
    );
    let confirmation_message = format!(
        "AI Configuration updated successfully!
Mode: `{:?}`
Targets: {}",
        mode,
        if mode == AiMode::Specific { targets_display } else { "N/A".to_string() }
    );
    
    let response_msg = CreateInteractionResponseMessage::new()
        .content(confirmation_message)
        .ephemeral(true);
    command.create_followup(&ctx.http, response_msg).await?;

    Ok(())
}

pub fn register() -> CreateCommand {
    CreateCommand::new("aiconfig")
        .description("Configure AI channel access for this server.")
        .default_member_permissions(Permissions::ADMINISTRATOR) // Restrict at API level too
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "mode", "Set AI interaction mode")
                .required(true)
                .add_string_choice("AI completely off", "off")
                .add_string_choice("AI active in all channels (global)", "global")
                .add_string_choice("AI active only in specific channels/categories", "specific"),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "targets", "Space-separated channel/category IDs or #mentions (for 'specific' mode)")
                .required(false), // Required only if mode is 'specific', handled in logic
        )
}
