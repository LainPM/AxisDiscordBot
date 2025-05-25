use serenity::all::CommandOptionType; // Removed ResolvedOption, ResolvedValue
use serenity::builder::{CreateCommand, CreateCommandOption, CreateInteractionResponseFollowup}; // Removed CreateInteractionResponse
// Corrected import path for CommandOptionValue:
use serenity::model::application::CommandInteraction;
use serenity::model::application::interaction::application_command::CommandOptionValue;
use serenity::model::id::GuildId; // Removed ChannelId as it's not used as a type
use serenity::model::permissions::Permissions;
use serenity::prelude::*;
use tracing::{error, info}; // Removed debug as it's not used

use crate::ai::config::{AiConfigStore, AiGuildConfig, AiMode}; // Assuming ai::config is now available

pub async fn run(ctx: &Context, command: &CommandInteraction) -> Result<(), serenity::Error> {
    command.defer_ephemeral(&ctx.http).await?;

    // 1. Check for administrator permissions
    let member_permissions = command.member.as_ref().map(|m| m.permissions).flatten();
    if !member_permissions.map_or(false, |p| p.contains(Permissions::ADMINISTRATOR)) {
        let followup_msg = CreateInteractionResponseFollowup::new()
            .content("You must be an administrator to use this command.")
            .ephemeral(true);
        command.create_followup(&ctx.http, followup_msg).await?;
        return Ok(());
    }

    let guild_id = match command.guild_id {
        Some(id) => id,
        None => {
            let followup_msg = CreateInteractionResponseFollowup::new()
                .content("This command can only be used in a server.")
                .ephemeral(true);
            command.create_followup(&ctx.http, followup_msg).await?;
            return Ok(());
        }
    };

    let mut mode_opt: Option<AiMode> = None;
    let mut targets_opt: Option<Vec<String>> = None;

    // 2. Parse command options
    for option_data in &command.data.options { // option_data is &CommandDataOption
        match option_data.name.as_str() { // Use .as_str() for matching
            "mode" => {
                if let CommandOptionValue::String(mode_str) = &option_data.value { // Destructure CommandOptionValue
                    mode_opt = match mode_str.as_str() { // mode_str is &String
                        "off" => Some(AiMode::Off),
                        "global" => Some(AiMode::Global),
                        "specific" => Some(AiMode::Specific),
                        _ => None,
                    };
                }
            }
            "targets" => {
                if let CommandOptionValue::String(targets_str) = &option_data.value { // Destructure CommandOptionValue
                    let parsed_targets: Vec<String> = targets_str // targets_str is &String
                        .split_whitespace()
                        .filter_map(|s| { // s is &str
                            // Try to parse as <#ID> or just ID
                            if s.starts_with("<#") && s.ends_with('>') {
                                Some(s[2..s.len()-1].to_string())
                            } else if s.parse::<u64>().is_ok() { // Basic check if it's a plain ID
                                Some(s.to_string())
                            } else {
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
            let followup_msg = CreateInteractionResponseFollowup::new()
                .content("Invalid mode selected.")
                .ephemeral(true);
            command.create_followup(&ctx.http, followup_msg).await?;
            return Ok(());
        }
    };

    if mode == AiMode::Specific && targets_opt.is_none() {
        let followup_msg = CreateInteractionResponseFollowup::new()
            .content("For 'specific' mode, you must provide target channels/categories.")
            .ephemeral(true);
        command.create_followup(&ctx.http, followup_msg).await?;
        return Ok(());
    }

    // 3. Retrieve AiConfiguration
    let data_read = ctx.data.read().await;
    let config_store_lock = match data_read.get::<AiConfigStore>() {
        Some(store) => store.clone(),
        None => {
            error!("AiConfigStore not found in TypeMap. Cannot configure AI.");
            let followup_msg = CreateInteractionResponseFollowup::new()
                .content("AI Configuration system is not available. Please contact bot support.")
                .ephemeral(true);
            command.create_followup(&ctx.http, followup_msg).await?;
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
            let followup_msg = CreateInteractionResponseFollowup::new()
                .content(format!("Error saving AI configuration: {}", e))
                .ephemeral(true);
            command.create_followup(&ctx.http, followup_msg).await?;
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
    
    let followup_msg = CreateInteractionResponseFollowup::new()
        .content(confirmation_message)
        .ephemeral(true);
    command.create_followup(&ctx.http, followup_msg).await?;

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
