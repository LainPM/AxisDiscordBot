// src/commands/ping.rs
use serenity::{
    builder::CreateCommand,
    model::prelude::*,
    prelude::*,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::bot::ShardManagerContainer; // ← Your TypeMapKey from bot.rs

/// The actual handler for `/ping`
pub async fn ping(ctx: &Context, command: &CommandInteraction) -> Result<(), serenity::Error> {
    // 1) Pull the ShardManager out of ctx.data
    let data_read = ctx.data.read().await;
    let manager_lock = data_read
        .get::<ShardManagerContainer>()
        .expect("Expected ShardManager in TypeMap")
        .clone();
    drop(data_read);

    // 2) Lock it and measure gateway latency
    let manager = manager_lock.lock().await;
    let gateway_ms = manager
        .shards
        .get(0)
        .and_then(|shard| shard.latency())
        .unwrap_or_default()
        .as_millis();

    // 3) Measure REST latency (defer + edit)
    let rest_start = std::time::Instant::now();
    command.defer(&ctx.http).await?;
    let rest_ms = rest_start.elapsed().as_millis();

    // 4) Edit the deferred response with both numbers
    command
        .edit_response(&ctx.http, |resp| {
            resp.content(format!("Gateway: {}ms\nREST:    {}ms", gateway_ms, rest_ms))
        })
        .await?;

    Ok(())
}

/// How this command shows up to Discord when you register it
pub fn register_ping() -> CreateCommand {
    CreateCommand::new("ping")
        .description("Check the bot's latency")
}
