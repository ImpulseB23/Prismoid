use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub enum Platform {
    Twitch,
    YouTube,
    Kick,
}

#[derive(Debug, Clone, Serialize)]
pub struct Badge {
    pub set_id: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnifiedMessage {
    pub id: String,
    pub platform: Platform,
    pub timestamp: i64,
    pub arrival_time: i64,
    pub username: String,
    pub display_name: String,
    pub platform_user_id: String,
    pub message_text: String,
    pub badges: Vec<Badge>,
    pub is_mod: bool,
    pub is_subscriber: bool,
    pub is_broadcaster: bool,
    pub color: Option<String>,
    pub reply_to: Option<String>,
}
