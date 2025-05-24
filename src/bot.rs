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

    fn cleanup_expired_conversations(&self) {
        let mut to_remove = Vec::new();
        
        for entry in self.active_conversations.iter() {
            if entry.value().is_expired(30) { // 30 minute timeout
                to_remove.push(*entry.key());
            }
        }

        for channel_id in to_remove {
            self.active_conversations.remove(&channel_id);
            debug!("Cleaned up expired conversation in channel {}", channel_id);
        }
    }

    fn start_conversation(&self, channel_id: ChannelId, user_id: UserId) {
        let state = ConversationState::new(user_id);
        self.active_conversations.insert(channel_id, state);
        info!("Started new conversation with user {} in channel {}", user_id, channel_id);
    }

    fn update_conversation(&self, channel_id: ChannelId, user_id: UserId) -> bool {
        if let Some(mut state) = self.active_conversations.get_mut(&channel_id) {
            if state.user_id == user_id {
                state.update_activity();
                return true;
            } else {
                // Different user trying to use the channel, end the old conversation
                debug!("Different user {} trying to use channel {}, ending old conversation", user_id, channel_id);
                self.active_conversations.remove(&channel_id);
                return false;
            }
        }
        false
    }

    fn end_conversation(&self, channel_id: ChannelId, user_id: UserId) -> bool {
        if let Some(state) = self.active_conversations.get(&channel_id) {
            if state.user_id == user_id {
                self.active_conversations.remove(&channel_id);
                info!("Ended conversation with user {} in channel {}", user_id, channel_id);
                return true;
            }
        }
        false
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
        
        // Don't auto-sync commands - use manual !sync_all command instead
        info!("Bot ready! Use !sync_all to manually sync slash commands.");

        // Start background task to cleanup expired conversations
        let conversations = self.active_conversations.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300)); // Check every 5 minutes
            loop {
                interval.tick().await;
                let mut to_remove = Vec::new();
                
                for entry in conversations.iter() {
                    if entry.value().is_expired(30) { // 30 minute timeout
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
        // Handle commands in a separate task to prevent blocking
        let ctx_clone = ctx.clone();
        let interaction_clone = interaction.clone();
        
        tokio::spawn(async move {
            if let Interaction::Command(command) = interaction_clone {
                info!("Processing command: {} from user: {}", command.data.name, command.user.tag());
                
                let result = match command.data.name.as_str() {
                    "ping" => {
                        debug!("Executing ping command");
                        commands::ping(&ctx_clone, &command).await
                    },
                    "serverinfo" => {
                        debug!("Executing serverinfo command");
                        commands::serverinfo(&ctx_clone, &command).await
                    },
                    "membercount" => {
                        debug!("Executing membercount command");
                        commands::membercount(&ctx_clone, &command).await
                    },
                    unknown => {
                        error!("Unknown command received: {}", unknown);
                        let response = CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .content("Unknown command.")
                                .ephemeral(true)
                        );
                        command.create_response(&ctx_clone.http, response).await
                    }
                };

                if let Err(e) = result {
                    error!("Error executing command {}: {}", command.data.name, e);
                    
                    // Try to send error response if we haven't responded yet
                    let error_response = CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content("An error occurred while processing the command.")
                            .ephemeral(true)
                    );
                    
                    if let Err(resp_err) = command.create_response(&ctx_clone.http, error_response).await {
                        error!("Failed to send error response: {}", resp_err);
                    }
                }
                
                info!("Completed processing command: {}", command.data.name);
            }
        });
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        // Handle sync commands first and return immediately
        if msg.content.trim() == "!sync_all" {
            info!("Sync command received from {}", msg.author.tag());
            
            // Check permissions in a separate task to not block other messages
            let ctx_clone = ctx.clone();
            let msg_clone = msg.clone();
            
            tokio::spawn(async move {
                // Check if user has admin permissions
                if let Some(guild_id) = msg_clone.guild_id {
                    match guild_id.member(&ctx_clone.http, msg_clone.author.id).await {
                        Ok(member) => {
                            if !member.permissions(&ctx_clone.cache).map_or(false, |p| p.administrator()) {
                                let _ = msg_clone.reply(&ctx_clone.http, "❌ You need Administrator permissions to sync commands.").await;
                                return;
                            }
                        },
                        Err(_) => {
                            let _ = msg_clone.reply(&ctx_clone.http, "❌ Unable to check permissions.").await;
                            return;
                        }
                    }
                } else {
                    let _ = msg_clone.reply(&ctx_clone.http, "❌ This command can only be used in servers.").await;
                    return;
                }

                let _ = msg_clone.reply(&ctx_clone.http, "🔄 Syncing commands... Please wait.").await;
                
                let register_commands = vec![
                    commands::register_ping(),
                    commands::register_serverinfo(),
                    commands::register_membercount(),
                ];
                
                // Add a small delay to be respectful to Discord's API
                tokio::time::sleep(Duration::from_millis(500)).await;
                
                match Command::set_global_commands(&ctx_clone.http, register_commands).await {
                    Ok(commands) => {
                        info!("Successfully synced {} slash commands", commands.len());
                        let _ = msg_clone.reply(&ctx_clone.http, &format!("✅ Successfully synced {} slash commands!", commands.len())).await;
                    },
                    Err(e) => {
                        error!("Failed to sync commands: {}", e);
                        let _ = msg_clone.reply(&ctx_clone.http, &format!("❌ Failed to sync commands: {}", e)).await;
                    }
                }
            });
            
            return; // Important: return immediately to not interfere with other processing
        }

        // Handle AI conversations in a separate task to prevent blocking
        let ctx_clone = ctx.clone();
        let msg_clone = msg.clone();
        let gemini_client = self.gemini_client.clone();
        let config = self.config.clone();
        let conversations = self.active_conversations.clone();
        
        tokio::spawn(async move {
            debug!("Processing AI message from {}: '{}' - Current conversation active: {}", 
                   msg_clone.author.tag(), msg_clone.content, 
                   conversations.get(&msg_clone.channel_id).map_or(false, |state| state.user_id == msg_clone.author.id));
            
            // Cleanup expired conversations periodically
            let mut to_remove = Vec::new();
            for entry in conversations.iter() {
                if entry.value().is_expired(30) {
                    to_remove.push(*entry.key());
                }
            }
            for channel_id in to_remove {
                conversations.remove(&channel_id);
            }

            let has_active_convo = conversations.get(&msg_clone.channel_id)
                .map_or(false, |state| state.user_id == msg_clone.author.id);

            // Check if user wants to stop an active conversation
            if has_active_convo {
                if gemini_client.should_stop_conversation(&msg_clone.content) {
                    if let Some(state) = conversations.get(&msg_clone.channel_id) {
                        if state.user_id == msg_clone.author.id {
                            conversations.remove(&msg_clone.channel_id);
                            info!("Ended conversation with user {} in channel {}", msg_clone.author.id, msg_clone.channel_id);
                            let _ = msg_clone.reply(&ctx_clone.http, "Conversation ended. Feel free to reach out again if you need assistance with Roblox development.").await;
                            return;
                        }
                    }
                } else {
                    // Update conversation activity
                    if let Some(mut state) = conversations.get_mut(&msg_clone.channel_id) {
                        if state.user_id == msg_clone.author.id {
                            state.update_activity();
                        }
                    }
                }
            }
            
            // Determine if bot should respond to this message
            let should_respond = if has_active_convo {
                true // Always respond to active conversations
            } else {
                // Check if this is a new conversation request
                gemini_client.should_respond_to_message(
                    &msg_clone.content,
                    &config.bot_name,
                    msg_clone.author.id,
                    msg_clone.channel_id,
                    &Arc::new(DashMap::new()),
                )
            };

            if should_respond {
                debug!("Bot will respond to message from {} in channel {}", msg_clone.author.tag(), msg_clone.channel_id);
                
                // Start new conversation if not already active
                if !has_active_convo {
                    debug!("Starting new conversation for user {} in channel {}", msg_clone.author.id, msg_clone.channel_id);
                    let state = ConversationState::new(msg_clone.author.id);
                    conversations.insert(msg_clone.channel_id, state);
                    info!("Started new conversation with user {} in channel {}", msg_clone.author.id, msg_clone.channel_id);
                }

                let _typing_guard = msg_clone.channel_id.start_typing(&ctx_clone.http);
                
                match gemini_client.generate_response(&msg_clone.content, &msg_clone.author, msg_clone.guild_id, &ctx_clone).await {
                    Ok(response) => {
                        debug!("Generated AI response for user {}", msg_clone.author.tag());
                        if let Err(e) = msg_clone.reply(&ctx_clone.http, response).await {
                            error!("Failed to send AI response: {}", e);
                            // End conversation on send failure to prevent getting stuck
                            conversations.remove(&msg_clone.channel_id);
                        }
                    }
                    Err(e) => {
                        error!("Failed to generate AI response: {}", e);
                        let fallback_message = if e.to_string().contains("timeout") {
                            "Request timed out. Please try again."
                        } else if e.to_string().contains("API error") {
                            "I'm experiencing technical difficulties. Please try again later."
                        } else {
                            "I'm having trouble processing your request right now."
                        };
                        
                        if let Err(send_err) = msg_clone.reply(&ctx_clone.http, fallback_message).await {
                            error!("Failed to send fallback AI response: {}", send_err);
                        }
                        
                        // End conversation on AI failure to prevent getting stuck
                        conversations.remove(&msg_clone.channel_id);
                    }
                }
            } else {
                debug!("Bot will not respond to message from {} in channel {}", msg_clone.author.tag(), msg_clone.channel_id);
            }
        });
    }
}
