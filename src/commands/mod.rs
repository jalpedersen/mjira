pub mod fields;
pub mod instance;
pub mod issue;
pub mod project;
pub mod query;
pub mod search;

use colored::Colorize;
use serde_json::Value;

/// Print a key-value row with aligned label
pub fn print_field(label: &str, value: &str) {
    println!("{:>14}  {}", label.bold(), value);
}

/// Extract a string nested in a JSON object: obj["field"]["name"] -> &str
pub fn field_name<'a>(obj: &'a Value, field: &str) -> &'a str {
    obj.get(field)
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("—")
}

/// Extract a display name from a user-like object
pub fn display_name<'a>(obj: &'a Value, field: &str) -> &'a str {
    obj.get(field)
        .and_then(|v| v.get("displayName"))
        .and_then(|v| v.as_str())
        .unwrap_or("—")
}

/// Truncate a string to max_len, appending "…" if truncated
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_len - 1).collect::<String>())
    }
}

/// Trim ISO-8601 datetime to just the date portion for compact display
pub fn short_date(dt: &str) -> &str {
    dt.get(..10).unwrap_or(dt)
}
