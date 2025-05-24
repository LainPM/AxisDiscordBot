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
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected and ready!", ready.user.name);
        info!("Bot ID: {}", ready.user.id);
        info!("Connected to {} guilds", ready.guilds.len());
        
        let register_commands = vec![
            commands::register_ping(),
            commands::register_serverinfo(),
            commands::register_membercount(),
        ];

        match Command::set_global_commands(&ctx.http, register_commands).await {
            Ok(commands) => info!("Successfully registered {} application commands", commands.len()),
            Err(e) => error!("Failed to register application commands: {}", e),
        }
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

        debug!("Received message from {}: {}", msg.author.tag(), msg.content);
        let http = ctx.http.clone();

        // Check if AI should stop conversation
        if let Some(active_user) = self.active_conversations.get(&msg.channel_id) {
            if *active_user.value() == msg.author.id {
                match self.gemini_client.should_stop_conversation(&msg.content, &msg.author).await {
                    Ok(true) => {
                        info!("Stopping conversation with {} in channel {}", msg.author.tag(), msg.channel_id);
                        self.active_conversations.remove(&msg.channel_id);
                        if let Err(e) = msg.reply(&http, "Conversation ended. Feel free to reach out again if you need assistance with Roblox development.").await {
                            error!("Failed to send stop confirmation: {}", e);
                        }
                        return;
                    },
                    Ok(false) => {},
                    Err(e) => {
                        error!("Failed to check conversation stop: {}", e);
                    }
                }
            }
        }
        
        let should_respond = self.gemini_client.should_respond_to_message(
            &msg.content,
            &self.config.bot_name,
            msg.author.id,
            msg.channel_id,
            &self.active_conversations,
        ).await.unwrap_or(false);

        if should_respond {
            info!("Responding to message from {} in channel {}", msg.author.tag(), msg.channel_id);
            let is_existing_active_convo_for_user = self.active_conversations.get(&msg.channel_id)
                .map_or(false, |user| *user.value() == msg.author.id);

            if !is_existing_active_convo_for_user {
                self.active_conversations.insert(msg.channel_id, msg.author.id);
                info!("Started new conversation with {} in channel {}", msg.author.tag(), msg.channel_id);
            }

            let _typing_guard = msg.channel_id.start_typing(&http);
            
            match self.gemini_client.generate_response(&msg.content, &msg.author).await {
                Ok(response) => {
                    debug!("Generated AI response: {}", response);
                    if let Err(e) = msg.reply(&http, response).await {
                        error!("Failed to send AI response: {}", e);
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
                    if let Err(e) = msg.reply(&http, fallback_message).await {
                        error!("Failed to send fallback AI response: {}", e);
                    }
                }
            }
        }
    }
}
