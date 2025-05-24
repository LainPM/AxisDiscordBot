use reqwest::Client;
use serde_json::{json, Value};
use anyhow::{Result, Context};
use dashmap::DashMap;
use serenity::model::id::{ChannelId, UserId, GuildId};
use serenity::model::prelude::User;
use std::sync::Arc;
use tracing::{error, debug, info};

pub mod intents;

pub struct GeminiClient {
    client: Client,
    api_key: String,
}

impl GeminiClient {
    pub fn new(api_key: String) -> Self {
        info!("Initializing Gemini AI client");
        Self {
            client: Client::new(),
            api_key,
        }
    }

    async fn get_user_info(&self, user: &User, guild_id: Option<GuildId>, ctx: &serenity::prelude::Context) -> String {
        let mut user_info = format!(
            "Username: {}\nUser ID: {}\nDisplay Name: {}",
            user.tag(),
            user.id,
            user.global_name.as_ref().unwrap_or(&user.name)
        );

        // Get avatar URL
        if let Some(avatar_url) = user.avatar_url() {
            user_info.push_str(&format!("\nAvatar: {}", avatar_url));
        }

        // Get nickname and member info if in a guild
        if let Some(guild_id) = guild_id {
            match guild_id.member(&ctx.http, user.id).await {
                Ok(member) => {
                    if let Some(nick) = &member.nick {
                        user_info.push_str(&format!("\nNickname: {}", nick));
                    }
                    
                    // Note: User bio/profile info is not easily accessible via standard bot APIs
                    // This would require special Discord permissions and may not work for most bots
                },
                Err(e) => debug!("Could not fetch member info: {}", e)
            }
        }

        user_info
    }

    pub async fn generate_response(&self, prompt: &str, user: &User, guild_id: Option<GuildId>, ctx: &serenity::prelude::Context) -> Result<String> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash-latest:generateContent?key={}",
            self.api_key
        );

        let user_info = self.get_user_info(user, guild_id, ctx).await;

        let system_prompt = format!(
            "You are Axis, a professional Discord bot designed specifically for Roblox development assistance. \
            Your role is to provide expert guidance on Roblox Studio, Luau scripting, game development patterns, \
            optimization techniques, and development best practices.\n\n\
            IMPORTANT GUIDELINES:\n\
            - Maintain a professional, serious tone at all times\n\
            - Never use emojis, especially happy or cheerful ones\n\
            - Be direct, clear, and technical in your responses\n\
            - Focus on providing accurate, actionable information\n\
            - Keep responses under 2000 characters due to Discord limits\n\
            - When providing code examples, use proper Luau syntax\n\
            - If you don't know something, state it directly rather than guessing\n\
            - Address the user by their username when appropriate\n\
            - You can reference user information like their avatar, nickname, and user ID when relevant\n\
            - Note: User bio information is not currently available through the bot API\n\n\
            Current user information:\n{}\n\n\
            User message: {}",
            user_info, prompt
        );

        let payload = json!({
            "contents": [{
                "parts": [{
                    "text": system_prompt
                }]
            }],
            "generationConfig": {
                "temperature": 0.3,
                "topK": 20,  
                "topP": 0.8,
                "maxOutputTokens": 1000,
            },
            "safetySettings": [
                {
                    "category": "HARM_CATEGORY_HARASSMENT",
                    "threshold": "BLOCK_MEDIUM_AND_ABOVE"
                },
                {
                    "category": "HARM_CATEGORY_HATE_SPEECH",
                    "threshold": "BLOCK_MEDIUM_AND_ABOVE"
                },
                {
                    "category": "HARM_CATEGORY_SEXUALLY_EXPLICIT",
                    "threshold": "BLOCK_MEDIUM_AND_ABOVE"
                },
                {
                    "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                    "threshold": "BLOCK_MEDIUM_AND_ABOVE"
                }
            ]
        });

        debug!("Sending request to Gemini API for response generation");
        
        let response = self.client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .context("Failed to send request to Gemini API")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!("Gemini API error {}: {}", status, error_text);
            return Err(anyhow::anyhow!("Gemini API error {}: {}", status, error_text));
        }

        let json: Value = response.json().await
            .context("Failed to parse Gemini API response")?;

        debug!("Successfully received response from Gemini API");

        // Extract the response text with better error handling
        let text = json["candidates"]
            .get(0)
            .and_then(|candidate| candidate["content"]["parts"].get(0))
            .and_then(|part| part["text"].as_str())
            .context("Invalid response structure from Gemini API")?
            .to_string();

        // Ensure Discord character limit compliance
        if text.len() > 2000 {
            info!("Response truncated from {} to 2000 characters", text.len());
            Ok(format!("{}...", &text[..1997]))
        } else {
            Ok(text)
        }
    }

    pub fn should_stop_conversation(&self, message: &str) -> bool {
        let message_lower = message.to_lowercase();
        let content_lower = message_lower.trim();
        
        // More reliable pattern matching for conversation endings
        let stop_patterns = [
            "bye", "goodbye", "see ya", "see you", "cya", "later",
            "that's all", "thats all", "i'm done", "im done", "done",
            "thanks that's all", "thanks thats all", "thank you that's all",
            "no more questions", "stop", "quit", "exit", "leave me alone",
            "end conversation", "nevermind", "never mind", "forget it"
        ];

        let explicit_stops = stop_patterns.iter().any(|&pattern| {
            content_lower == pattern || 
            content_lower.starts_with(&format!("{} ", pattern)) ||
            content_lower.ends_with(&format!(" {}", pattern))
        });

        // Check for final thanks without questions
        let is_final_thanks = (content_lower.contains("thank") || content_lower.contains("thx") || content_lower.contains(" ty ")) 
            && !content_lower.contains("?") 
            && !content_lower.contains("how") 
            && !content_lower.contains("what") 
            && !content_lower.contains("can you")
            && !content_lower.contains("help")
            && content_lower.len() < 50; // Short thanks messages

        debug!("Stop conversation analysis - explicit: {}, final_thanks: {}", explicit_stops, is_final_thanks);
        explicit_stops || is_final_thanks
    }

    pub fn should_respond_to_message(
        &self,
        content: &str,
        bot_name: &str,
        author_id: UserId,
        channel_id: ChannelId,
        active_conversations: &Arc<DashMap<ChannelId, UserId>>,
    ) -> bool {
        // If there's an active conversation with this user in this channel, always respond
        if let Some(active_user_id) = active_conversations.get(&channel_id) {
            if *active_user_id == author_id {
                debug!("Responding due to active conversation with user {}", author_id);
                return true;
            }
        }

        // Reliable keyword-based detection for new conversations
        let content_lower_string = content.to_lowercase();
        let content_lower = content_lower_string.trim();
        let bot_name_lower = bot_name.to_lowercase();
        
        // Direct bot mentions and greetings
        let direct_mentions = [
            &bot_name_lower,
            &format!("hey {}", bot_name_lower),
            &format!("hi {}", bot_name_lower), 
            &format!("hello {}", bot_name_lower),
            &format!("{} help", bot_name_lower),
            &format!("help {}", bot_name_lower),
        ];

        // Roblox/development keywords
        let dev_keywords = [
            "roblox", "luau", "script", "scripting", "studio", "rbx", "remote event",
            "remote function", "datastore", "leaderstats", "gui", "screengu",
            "local script", "server script", "game development", "rbxasset"
        ];

        // Help request patterns
        let help_patterns = [
            "help me", "can you help", "i need help", "how do i", "how to",
            "what is", "explain", "show me", "teach me", "can you",
            "do you know", "question about"
        ];

        let has_direct_mention = direct_mentions.iter().any(|&mention| content_lower.contains(mention));
        let has_dev_keyword = dev_keywords.iter().any(|&keyword| content_lower.contains(keyword));
        let has_help_request = help_patterns.iter().any(|&pattern| content_lower.contains(pattern));

        let should_respond = has_direct_mention || (has_help_request && has_dev_keyword) || 
            (content_lower.len() > 10 && has_dev_keyword && content_lower.contains("?"));

        debug!("Response decision - direct: {}, dev: {}, help: {}, result: {}", 
               has_direct_mention, has_dev_keyword, has_help_request, should_respond);
        
        should_respond
    }
}
