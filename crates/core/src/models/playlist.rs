use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    /// "curated" (shipped with the catalog, read-only) or "user".
    pub kind: String,
    pub description: Option<String>,
    /// Number of grid cards in this playlist (merged multi-language groups
    /// count once, matching what the filtered Browse view shows).
    pub game_count: i64,
}
