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
        if let Interaction::Command(command) = interaction {
            info!("Received command: {}", command.data.name);
            let result = match command.data.name.as_str() {
                "ping" => commands::ping(&ctx, &command).await,
                "serverinfo" => commands::serverinfo(&ctx, &command).await,
                "membercount" => commands::membercount(&ctx, &command).await,
                _ => {
                    error!("Unknown command: {}", command.data.name);
                    Ok(())
                }
            };

            if let Err(e) = result {
                error!("Error handling command {}: {}", command.data.name, e);
                let response = CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new()
                        .content("An error occurred while processing the command.")
                        .ephemeral(true)
                );
                let _ = command.create_response(&ctx.http, response).await;
            }
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        // Handle sync commands first (before other processing)
        if msg.content.starts_with("!sync_all") {
            info!("Sync command received from {}", msg.author.tag());
            
            // Check if user has admin permissions
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

            let _ = msg.reply(&ctx.http, "🔄 Syncing commands... Please wait.").await;
            
            let register_commands = vec![
                commands::register_ping(),
                commands::register_serverinfo(),
                commands::register_membercount(),
            ];
            
            // Add a small delay to be respectful to Discord's API
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
            return;
        }

        debug!("Received message from {}: '{}' - Current conversation active: {}", 
               msg.author.tag(), msg.content, self.has_active_conversation(msg.channel_id, msg.author.id));
        
        // Cleanup expired conversations periodically
        self.cleanup_expired_conversations();

        let http = ctx.http.clone();

        // Check if user wants to stop an active conversation
        if self.has_active_conversation(msg.channel_id, msg.author.id) {
            if self.gemini_client.should_stop_conversation(&msg.content) {
                if self.end_conversation(msg.channel_id, msg.author.id) {
                    if let Err(e) = msg.reply(&http, "Conversation ended. Feel free to reach out again if you need assistance with Roblox development.").await {
                        error!("Failed to send stop confirmation: {}", e);
                    }
                    return;
                }
            } else {
                // Update conversation activity
                self.update_conversation(msg.channel_id, msg.author.id);
            }
        }
        
        // Determine if bot should respond to this message
        let should_respond = if self.has_active_conversation(msg.channel_id, msg.author.id) {
            true // Always respond to active conversations
        } else {
            // Check if this is a new conversation request
            self.gemini_client.should_respond_to_message(
                &msg.content,
                &self.config.bot_name,
                msg.author.id,
                msg.channel_id,
                &Arc::new(DashMap::new()), // Pass empty map since we're managing state differently now
            )
        };

        if should_respond {
            debug!("Bot will respond to message from {} in channel {}", msg.author.tag(), msg.channel_id);
            
            // Start new conversation if not already active
            if !self.has_active_conversation(msg.channel_id, msg.author.id) {
                debug!("Starting new conversation for user {} in channel {}", msg.author.id, msg.channel_id);
                self.start_conversation(msg.channel_id, msg.author.id);
            }

            let _typing_guard = msg.channel_id.start_typing(&http);
            
            match self.gemini_client.generate_response(&msg.content, &msg.author, msg.guild_id, &ctx).await {
                Ok(response) => {
                    debug!("Generated AI response: {}", response);
                    if let Err(e) = msg.reply(&http, response).await {
                        error!("Failed to send AI response: {}", e);
                        // End conversation on send failure to prevent getting stuck
                        self.end_conversation(msg.channel_id, msg.author.id);
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
                    
                    if let Err(send_err) = msg.reply(&http, fallback_message).await {
                        error!("Failed to send fallback AI response: {}", send_err);
                    }
                    
                    // End conversation on AI failure to prevent getting stuck
                    self.end_conversation(msg.channel_id, msg.author.id);
                }
            }
        } else {
            debug!("Bot will not respond to message from {} in channel {}", msg.author.tag(), msg.channel_id);
        }
    }
}
