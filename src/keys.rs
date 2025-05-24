use serenity::prelude::*;
use serenity::client::bridge::gateway::ShardManager;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A zero-sized type to use as our TypeMap key
pub struct ShardManagerContainer;

impl TypeMapKey for ShardManagerContainer {
    type Value = Arc<Mutex<ShardManager>>;
}
