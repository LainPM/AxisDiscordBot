use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serenity::model::id::GuildId;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, ErrorKind};
// use std::path::Path; // Removed unused import
use std::sync::Arc;
use tokio::sync::RwLock; // Changed from std::sync::RwLock for async contexts if needed by Serenity TypeMap access

pub const CONFIG_PATH: &str = "ai_config.json";

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiMode {
    Off,
    Global,
    Specific,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AiGuildConfig {
    pub mode: AiMode,
    pub allowed_ids: Vec<String>, // Stores ChannelId or CategoryId as strings
}

impl Default for AiGuildConfig {
    fn default() -> Self {
        Self {
            mode: AiMode::Off, // Default to off
            allowed_ids: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct AiConfiguration {
    // Using DashMap for concurrent reads/writes if individual guild configs are frequently updated.
    // If config is mostly read and written entirely on save, HashMap might be simpler.
    // For now, DashMap is fine.
    // #[serde(with = "dashmap::serde::preserve_order")] // Removed this line
    pub guilds: DashMap<GuildId, AiGuildConfig>,
}

impl AiConfiguration {
    pub fn load() -> Self {
        match File::open(CONFIG_PATH) {
            Ok(file) => {
                let reader = BufReader::new(file);
                serde_json::from_reader(reader).unwrap_or_else(|e| {
                    tracing::warn!("Failed to parse {}: {}. Using default.", CONFIG_PATH, e);
                    Default::default()
                })
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                tracing::info!("{} not found. Creating default config.", CONFIG_PATH);
                Default::default()
            }
            Err(e) => {
                tracing::warn!("Failed to open {}: {}. Using default.", CONFIG_PATH, e);
                Default::default()
            }
        }
    }

    pub fn save(&self) -> Result<(), anyhow::Error> {
        let file = OpenOptions::new().write(true).create(true).truncate(true).open(CONFIG_PATH)?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, self)?;
        tracing::info!("Successfully saved AI configuration to {}", CONFIG_PATH);
        Ok(())
    }

    // Helper to get a specific guild's config, or default if not found
    pub fn get_guild_config(&self, guild_id: &GuildId) -> AiGuildConfig {
        self.guilds.get(guild_id).map(|conf| conf.value().clone()).unwrap_or_default()
    }

    // Helper to update/insert a guild's config
    pub fn set_guild_config(&self, guild_id: GuildId, config: AiGuildConfig) {
        self.guilds.insert(guild_id, config);
    }
}

// TypeMapKey for global access via Serenity's context.data
pub struct AiConfigStore;

impl serenity::prelude::TypeMapKey for AiConfigStore {
    // Using Arc<RwLock<AiConfiguration>> for shared, mutable access
    type Value = Arc<RwLock<AiConfiguration>>;
}
