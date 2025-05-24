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
        Self {
            client: Client::new(),
            api_key,
        }
    }

    pub async fn generate_response(&self, prompt: &str, user_info: &str) -> Result<String> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash-latest:generateContent?key={}",
            self.api_key
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
            - If you don't know something, state it directly rather than guessing\n\n\
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

        debug!("Sending request to Gemini API");
        
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

        debug!("Received response from Gemini API");

        // Extract the response text
        let text = json["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .context("Invalid response structure from Gemini API")?
            .to_string();

        // Ensure Discord character limit compliance
        if text.len() > 2000 {
            Ok(format!("{}...", &text[..1997]))
        } else {
            Ok(text)
        }
    }

    pub async fn should_stop_conversation(&self, message: &str, user_info: &str) -> Result<bool> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash-latest:generateContent?key={}",
            self.api_key
        );

        let system_prompt = format!(
            "Analyze the following message to determine if the user wants to end the conversation. \
            Consider context clues like:\n\
            - Explicit goodbye statements (bye, goodbye, see you later, etc.)\n\
            - Statements indicating they're done (that's all, I'm finished, no more questions, etc.)\n\
            - Thank you messages that seem final\n\
            - Clear dismissal statements\n\n\
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

        let response = self.client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await
            .context("Failed to send conversation analysis request")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Failed to analyze conversation intent"));
        }

        let json: Value = response.json().await
            .context("Failed to parse conversation analysis response")?;

        let response_text = json["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("NO")
            .trim()
            .to_uppercase();

        Ok(response_text == "YES")
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
                debug!("Responding due to active conversation");
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
            - Direct mentions of the bot name\n\
            - Greetings directed at the bot (hey {}, hi {}, etc.)\n\
            - Requests for help or assistance\n\
            - Questions about Roblox development\n\
            - General programming or scripting questions\n\n\
            Message: {}\n\n\
            Respond with only 'YES' if the bot should respond, or 'NO' if it should not.",
            bot_name, bot_name, bot_name, content
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

        let response = self.client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .context("Failed to analyze message intent")?;

        if !response.status().is_success() {
            // Fallback to simple keyword detection if AI fails
            debug!("AI analysis failed, falling back to keyword detection");
            return Ok(self.fallback_should_respond(content, bot_name));
        }

        let json: Value = response.json().await
            .context("Failed to parse message intent response")?;

        let response_text = json["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("NO")
            .trim()
            .to_uppercase();

        let should_respond = response_text == "YES";
        debug!("AI determined should_respond: {}", should_respond);
        
        Ok(should_respond)
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
        ];
        
        triggers.iter().any(|trigger| content_lower.contains(trigger))
    }
}
