use reqwest::Client;
use serde_json::{json, Value};
use anyhow::{Result, Context};
use dashmap::DashMap;
use serenity::model::id::{ChannelId, UserId};
use std::sync::Arc;
use tracing::{error, debug, info};

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

    pub async fn generate_response(&self, prompt: &str, user: &serenity::model::prelude::User) -> Result<String> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash-latest:generateContent?key={}",
            self.api_key
        );

        let user_info = format!(
            "Username: {}, User ID: {}, Display Name: {}",
            user.tag(),
            user.id,
            user.global_name.as_ref().unwrap_or(&user.name)
        );

        let user_info = format!(
            "Username: {}, User ID: {}",
            user.tag(),
            user.id
        );

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
            - Address the user by their username when appropriate\n\n\
            Current user information: {}\n\n\
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

    pub async fn should_stop_conversation(&self, message: &str, user: &serenity::model::prelude::User) -> Result<bool> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash-latest:generateContent?key={}",
            self.api_key
        );

        let system_prompt = format!(
            "Analyze the following message to determine if the user wants to end the conversation. \
            Consider context clues like:\n\
            - Explicit goodbye statements (bye, goodbye, see you later, thanks that's all, etc.)\n\
            - Statements indicating they're done (that's all, I'm finished, no more questions, done, etc.)\n\
            - Thank you messages that seem final (thanks, thank you with no follow-up question)\n\
            - Clear dismissal statements (stop, quit, exit, leave, etc.)\n\n\
            User info: {}\n\
            Message to analyze: {}\n\n\
            Respond with only 'YES' if they want to end the conversation, or 'NO' if they want to continue.",
            user_info, message
        );

        let payload = json!({
            "contents": [{
                "parts": [{
                    "text": system_prompt
                }]
            }],
            "generationConfig": {
                "temperature": 0.1,
                "topK": 1,
                "topP": 0.1,
                "maxOutputTokens": 10,
            }
        });

        debug!("Analyzing conversation termination intent");

        let response = self.client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await
            .context("Failed to send conversation analysis request")?;

        if !response.status().is_success() {
            debug!("Conversation analysis API call failed, defaulting to continue");
            return Ok(false);
        }

        let json: Value = response.json().await
            .context("Failed to parse conversation analysis response")?;

        let response_text = json["candidates"]
            .get(0)
            .and_then(|candidate| candidate["content"]["parts"].get(0))
            .and_then(|part| part["text"].as_str())
            .unwrap_or("NO")
            .trim()
            .to_uppercase();

        let should_stop = response_text == "YES";
        debug!("Conversation termination analysis result: {}", should_stop);
        
        Ok(should_stop)
    }

    pub async fn should_respond_to_message(
        &self,
        content: &str,
        bot_name: &str,
        author_id: UserId,
        channel_id: ChannelId,
        active_conversations: &Arc<DashMap<ChannelId, UserId>>,
    ) -> Result<bool> {
        // If there's an active conversation with this user in this channel, always respond
        if let Some(active_user_id) = active_conversations.get(&channel_id) {
            if *active_user_id == author_id {
                debug!("Responding due to active conversation with user {}", author_id);
                return Ok(true);
            }
        }

        // Use AI to determine if the message is directed at the bot
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash-latest:generateContent?key={}",
            self.api_key
        );

        let system_prompt = format!(
            "Analyze this message to determine if it's directed at a bot named '{}' or requesting help with Roblox development.\n\
            Look for:\n\
            - Direct mentions of the bot name ({})\n\
            - Greetings directed at the bot (hey {}, hi {}, hello {}, etc.)\n\
            - Requests for help or assistance\n\
            - Questions about Roblox development, scripting, or game development\n\
            - General programming or scripting questions\n\
            - Questions that seem to be asking for technical assistance\n\n\
            Message: {}\n\n\
            Respond with only 'YES' if the bot should respond, or 'NO' if it should not.",
            bot_name, bot_name, bot_name, bot_name, bot_name, content
        );

        let payload = json!({
            "contents": [{
                "parts": [{
                    "text": system_prompt
                }]
            }],
            "generationConfig": {
                "temperature": 0.1,
                "topK": 1,
                "topP": 0.1,
                "maxOutputTokens": 10,
            }
        });

        debug!("Analyzing message intent for response decision");

        let response = self.client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<Value>().await {
                    Ok(json) => {
                        let response_text = json["candidates"]
                            .get(0)
                            .and_then(|candidate| candidate["content"]["parts"].get(0))
                            .and_then(|part| part["text"].as_str())
                            .unwrap_or("NO")
                            .trim()
                            .to_uppercase();

                        let should_respond = response_text == "YES";
                        debug!("AI determined should_respond: {}", should_respond);
                        Ok(should_respond)
                    },
                    Err(_) => {
                        debug!("Failed to parse AI response, using fallback");
                        Ok(self.fallback_should_respond(content, bot_name))
                    }
                }
            },
            _ => {
                debug!("AI analysis failed, using fallback keyword detection");
                Ok(self.fallback_should_respond(content, bot_name))
            }
        }
    }

    fn fallback_should_respond(&self, content: &str, bot_name: &str) -> bool {
        let content_lower = content.to_lowercase().trim().to_string();
        let bot_name_lower = bot_name.to_lowercase();
        
        let triggers = [
            format!("hey {}", bot_name_lower),
            format!("hi {}", bot_name_lower),
            format!("hello {}", bot_name_lower),
            format!("{} help", bot_name_lower),
            format!("help {}", bot_name_lower),
            bot_name_lower.clone(),
            "roblox".to_string(),
            "luau".to_string(),
            "script".to_string(),
            "scripting".to_string(),
            "help me".to_string(),
            "can you".to_string(),
        ];
        
        let should_respond = triggers.iter().any(|trigger| content_lower.contains(trigger));
        debug!("Fallback keyword detection result: {}", should_respond);
        should_respond
    }
}
