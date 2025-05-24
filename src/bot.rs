use serenity::async_trait;
use serenity::builder::{CreateInteractionResponse, CreateInteractionResponseMessage};
use serenity::client::{Context, EventHandler};
use serenity::model::gateway::Ready;
use serenity::model::id::{ChannelId, UserId};
use serenity::model::prelude::*;
use serenity::prelude::*;
use std::sync::Arc;
use dashmap::DashMap;
use tracing::{error, info, debug};
use std::time::{Duration, Instant};

use crate::ai::GeminiClient;
use crate::commands;
use crate::config::Config;

pub struct ShardManagerContainer;

impl TypeMapKey for ShardManagerContainer {
    type Value = Arc<serenity::gateway::ShardManager>;
}

#[derive(Debug, Clone)]
pub struct ConversationState {
    pub user_id: UserId,
    pub last_activity: Instant,
}

impl ConversationState {
    pub fn new(user_id: UserId) -> Self {
        Self {
            user_id,
            last_activity: Instant::now(),
        }
    }

    pub fn update_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn is_expired(&self, timeout_minutes: u64) -> bool {
        self.last_activity.elapsed() > Duration::from_secs(timeout_minutes * 60)
    }
}

pub struct Handler {
    pub config: Config,
    pub gemini_client: GeminiClient,
    pub active_conversations: Arc<DashMap<ChannelId, ConversationState>>,
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

    fn has_active_conversation(&self, channel_id: ChannelId, user_id: UserId) -> bool {
        self.active_conversations.get(&channel_id)
            .map_or(false, |state| state.user_id == user_id)
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected and ready!", ready.user.name);
        info!("Bot ID: {}", ready.user.id);
        info!("Connected to {} guilds", ready.guilds.len());
        info!("Bot ready! Use !sync_all to manually sync slash commands.");

        // Start background cleanup task
        let conversations = self.active_conversations.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                let mut to_remove = Vec::new();
                
                for entry in conversations.iter() {
                    if entry.value().is_expired(30) {
                        to_remove.push(*entry.key());
                    }
                }

                for channel_id in to_remove {
                    conversations.remove(&channel_id);
                    debug!("Background cleanup: removed expired conversation in channel {}", channel_id);
                }
            }
        });
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(command) = interaction {
            info!("Processing slash command: {} from user: {}", command.data.name, command.user.tag());
            
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
                unknown => {
                    error!("Unknown slash command: {}", unknown);
                    let response = CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content("Unknown command.")
                            .ephemeral(true)
                    );
                    command.create_response(&ctx.http, response).await
                }
            };

            match result {
                Ok(_) => info!("Successfully processed slash command: {}", command.data.name),
                Err(e) => {
                    error!("Error processing slash command {}: {}", command.data.name, e);
                    
                    let error_response = CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content("An error occurred while processing the command.")
                            .ephemeral(true)
                    );
                    
                    let _ = command.create_response(&ctx.http, error_response).await;
                }
            }
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        debug!("Received message from {}: '{}'", msg.author.tag(), msg.content);

        // Handle sync command immediately and return
        if msg.content.trim() == "!sync_all" {
            info!("Processing sync command from {}", msg.author.tag());
            
            // Check admin permissions
            if let Some(guild_id) = msg.guild_id {
                match guild_id.member(&ctx.http, msg.author.id).await {
                    Ok(member) => {
                        if !member.permissions(&ctx.cache).map_or(false, |p| p.administrator()) {
                            let _ = msg.reply(&ctx.http, "❌ You need Administrator permissions to sync commands.").await;
                            return;
                        }
                    },
                    Err(_) => {
                        let _ = msg.reply(&ctx.http, "❌ Unable to check permissions.").await;
                        return;
                    }
                }
            } else {
                let _ = msg.reply(&ctx.http, "❌ This command can only be used in servers.").await;
                return;
            }

            let _ = msg.reply(&ctx.http, "🔄 Syncing commands...").await;
            
            let register_commands = vec![
                commands::register_ping(),
                commands::register_serverinfo(),
                commands::register_membercount(),
            ];
            
            tokio::time::sleep(Duration::from_millis(500)).await;
            
            match Command::set_global_commands(&ctx.http, register_commands).await {
                Ok(commands) => {
                    info!("Successfully synced {} slash commands", commands.len());
                    let _ = msg.reply(&ctx.http, &format!("✅ Successfully synced {} slash commands!", commands.len())).await;
                },
                Err(e) => {
                    error!("Failed to sync commands: {}", e);
                    let _ = msg.reply(&ctx.http, &format!("❌ Failed to sync commands: {}", e)).await;
                }
            }
            return; // Critical: return immediately after sync
        }

        // Clean up expired conversations
        let mut to_remove = Vec::new();
        for entry in self.active_conversations.iter() {
            if entry.value().is_expired(30) {
                to_remove.push(*entry.key());
            }
        }
        for channel_id in to_remove {
            self.active_conversations.remove(&channel_id);
        }

        let has_active_convo = self.has_active_conversation(msg.channel_id, msg.author.id);

        // Check if user wants to stop conversation
        if has_active_convo && self.gemini_client.should_stop_conversation(&msg.content) {
            self.active_conversations.remove(&msg.channel_id);
            info!("Ended conversation with user {} in channel {}", msg.author.id, msg.channel_id);
            let _ = msg.reply(&ctx.http, "Conversation ended. Feel free to reach out again if you need assistance with Roblox development.").await;
            return;
        }

        // Update conversation activity if active
        if has_active_convo {
            if let Some(mut state) = self.active_conversations.get_mut(&msg.channel_id) {
                if state.user_id == msg.author.id {
                    state.update_activity();
                }
            }
        }

        // Determine if bot should respond
        let should_respond = if has_active_convo {
            true
        } else {
            self.gemini_client.should_respond_to_message(
                &msg.content,
                &self.config.bot_name,
                msg.author.id,
                msg.channel_id,
                &Arc::new(DashMap::new()),
            )
        };

        if should_respond {
            info!("Responding to message from {} in channel {}", msg.author.tag(), msg.channel_id);
            
            // Start new conversation if needed
            if !has_active_convo {
                let state = ConversationState::new(msg.author.id);
                self.active_conversations.insert(msg.channel_id, state);
                info!("Started new conversation with user {} in channel {}", msg.author.id, msg.channel_id);
            }

            // Show typing indicator
            let _typing = msg.channel_id.start_typing(&ctx.http);
            
            // Generate AI response
            match self.gemini_client.generate_response(&msg.content, &msg.author, msg.guild_id, &ctx).await {
                Ok(response) => {
                    debug!("Generated AI response for user {}", msg.author.tag());
                    if let Err(e) = msg.reply(&ctx.http, response).await {
                        error!("Failed to send AI response: {}", e);
                        self.active_conversations.remove(&msg.channel_id);
                    }
                }
                Err(e) => {
                    error!("Failed to generate AI response: {}", e);
                    let fallback = if e.to_string().contains("timeout") {
                        "Request timed out. Please try again."
                    } else {
                        "I'm having trouble processing your request right now."
                    };
                    
                    let _ = msg.reply(&ctx.http, fallback).await;
                    self.active_conversations.remove(&msg.channel_id);
                }
            }
        } else {
            debug!("Not responding to message from {}", msg.author.tag());
        }
    }
}
