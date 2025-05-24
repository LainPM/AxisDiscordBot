use serenity::async_trait;
use serenity::builder::{CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage, CreateEmbed};
use serenity::client::{Context, EventHandler};
use serenity::model::gateway::Ready;
use serenity::model::id::{ChannelId, UserId};
use serenity::model::prelude::*;
use serenity::prelude::*;
use std::sync::Arc;
use dashmap::DashMap;
use tracing::{error, info, debug, warn};
use chrono::Utc;

use crate::ai::GeminiClient;
use crate::commands;
use crate::config::Config;

pub struct ShardManagerContainer;

impl TypeMapKey for ShardManagerContainer {
    type Value = Arc<serenity::gateway::ShardManager>;
}

pub struct Handler {
    pub config: Config,
    pub gemini_client: GeminiClient,
    pub active_conversations: Arc<DashMap<ChannelId, UserId>>,
}

impl Handler {
    pub fn new(config: Config) -> Self {
        info!("Creating new Handler instance");
        let gemini_client = GeminiClient::new(config.gemini_api_key.clone());
        Self {
            config,
            gemini_client,
            active_conversations: Arc::new(DashMap::new()),
        }
    }

    async fn get_user_info(&self, ctx: &Context, user: &User, guild_id: Option<GuildId>) -> String {
        let mut info = format!("Username: {}", user.name);
        info.push_str(&format!(", Display Name: {}", user.global_name.as_ref().unwrap_or(&user.name)));
        info.push_str(&format!(", User ID: {}", user.id));
        
        if let Some(guild_id) = guild_id {
            if let Ok(nickname) = user.nick_in(&ctx.http, guild_id).await {
                if let Some(nick) = nickname {
                    info.push_str(&format!(", Nickname: {}", nick));
                }
            }
        }
        
        if let Some(avatar_url) = user.avatar_url() {
            info.push_str(&format!(", Avatar: {}", avatar_url));
        }
        
        info
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("Bot {} is connected and ready!", ready.user.name);
        info!("Bot ID: {}", ready.user.id);
        info!("Connected to {} guilds", ready.guilds.len());
        
        let register_commands = vec![
            commands::register_ping(),
            commands::register_serverinfo(),
            commands::register_membercount(),
        ];

        match Command::set_global_commands(&ctx.http, register_commands).await {
            Ok(commands) => {
                info!("Successfully registered {} application commands", commands.len());
                for cmd in commands {
                    info!("Registered command: {}", cmd.name);
                }
            },
            Err(e) => {
                error!("Failed to register application commands: {}", e);
            }
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(command) = interaction {
            info!("Received slash command: {} from user: {}", command.data.name, command.user.tag());
            
            let result = match command.data.name.as_str() {
                "ping" => {
                    debug!("Executing ping command");
                    commands::ping(&ctx, &command).await
                },
                "serverinfo" => {
                    debug!("Executing serverinfo command");
                    commands::serverinfo(&ctx, &command).await
                },
                "membercount" => {
                    debug!("Executing membercount command");
                    commands::membercount(&ctx, &command).await
                },
                _ => {
                    error!("Unknown command received: {}", command.data.name);
                    let response = CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content("Unknown command.")
                            .ephemeral(true)
                    );
                    command.create_response(&ctx.http, response).await
                }
            };

            if let Err(e) = result {
                error!("Error handling command {}: {}", command.data.name, e);
                let error_response = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("An error occurred while processing the command.")
                        .ephemeral(true)
                );
                if let Err(response_error) = command.create_response(&ctx.http, error_response).await {
                    error!("Failed to send error response: {}", response_error);
                }
            }
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            debug!("Ignoring bot message from {}", msg.author.tag());
            return;
        }

        info!("Received message from {}: '{}'", msg.author.tag(), msg.content);
        let http = ctx.http.clone();

        // Check if AI should determine the response (stop conversation, etc.)
        let user_info = self.get_user_info(&ctx, &msg.author, msg.guild_id).await;
        
        // First, check if we should stop the conversation using AI
        if let Some(active_user) = self.active_conversations.get(&msg.channel_id) {
            if *active_user.value() == msg.author.id {
                match self.gemini_client.should_stop_conversation(&msg.content, &user_info).await {
                    Ok(true) => {
                        info!("AI determined to stop conversation with {} in channel {}", msg.author.tag(), msg.channel_id);
                        self.active_conversations.remove(&msg.channel_id);
                        if let Err(e) = msg.reply(&http, "Conversation ended. Feel free to reach out again if you need assistance with Roblox development.").await {
                            error!("Failed to send stop confirmation: {}", e);
                        }
                        return;
                    },
                    Ok(false) => {
                        debug!("AI determined to continue conversation");
                    },
                    Err(e) => {
                        warn!("Failed to check if conversation should stop: {}", e);
                    }
                }
            }
        }
        
        // Check if we should respond to this message
        let should_respond = self.gemini_client.should_respond_to_message(
            &msg.content,
            &self.config.bot_name,
            msg.author.id,
            msg.channel_id,
            &self.active_conversations,
        ).await.unwrap_or(false);

        if should_respond {
            info!("Generating response for message from {} in channel {}", msg.author.tag(), msg.channel_id);
            
            // Track active conversation
            let is_existing_conversation = self.active_conversations.get(&msg.channel_id)
                .map_or(false, |user| *user.value() == msg.author.id);

            if !is_existing_conversation {
                self.active_conversations.insert(msg.channel_id, msg.author.id);
                info!("Started new conversation with {} in channel {}", msg.author.tag(), msg.channel_id);
            }

            // Show typing indicator
            let _typing_guard = msg.channel_id.start_typing(&http);
            
            match self.gemini_client.generate_response(&msg.content, &user_info).await {
                Ok(response) => {
                    info!("Generated AI response of length: {}", response.len());
                    debug!("AI Response: {}", response);
                    
                    if let Err(e) = msg.reply(&http, response).await {
                        error!("Failed to send AI response: {}", e);
                    } else {
                        info!("Successfully sent AI response to {}", msg.author.tag());
                    }
                }
                Err(e) => {
                    error!("Failed to generate AI response for {}: {}", msg.author.tag(), e);
                    let fallback_message = if e.to_string().contains("timeout") {
                        "Request timed out. Please try again."
                    } else if e.to_string().contains("API") {
                        "I'm experiencing technical difficulties. Please try again later."
                    } else {
                        "I'm having trouble processing your request right now."
                    };
                    
                    if let Err(send_error) = msg.reply(&http, fallback_message).await {
                        error!("Failed to send fallback response: {}", send_error);
                    }
                }
            }
        } else {
            debug!("Not responding to message from {} (doesn't meet response criteria)", msg.author.tag());
        }
    }

    async fn guild_create(&self, _ctx: Context, guild: Guild, _is_new: bool) {
        info!("Joined guild: {} (ID: {}, Members: {})", guild.name, guild.id, guild.member_count);
    }

    async fn guild_delete(&self, _ctx: Context, incomplete: UnavailableGuild, _full: Option<Guild>) {
        info!("Left guild: {}", incomplete.id);
    }
}
