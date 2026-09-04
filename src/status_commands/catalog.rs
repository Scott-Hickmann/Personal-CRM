pub(super) fn priority(kind: &str) -> usize {
    match kind {
        "contacts" => 0,
        "imessage" => 1,
        "whatsapp" => 2,
        "apple_calls" => 3,
        "whatsapp_calls" => 4,
        "gmail" => 5,
        "scoring" => 6,
        "photos" => 7,
        "google_publish" => 8,
        "suggestions" => 9,
        _ => usize::MAX,
    }
}

pub(super) fn label(kind: &str) -> &'static str {
    match kind {
        "contacts" => "Contacts",
        "imessage" => "iMessage",
        "whatsapp" => "WhatsApp",
        "apple_calls" => "Apple calls",
        "whatsapp_calls" => "WhatsApp calls",
        "gmail" => "Gmail",
        "scoring" => "Scoring",
        "photos" => "Photos",
        "google_publish" => "Google publish",
        "suggestions" => "Suggestions",
        _ => "Unknown",
    }
}

pub(super) fn downstream(kind: &str) -> Vec<&'static str> {
    match kind {
        "contacts" => vec!["scoring", "suggestions", "google_publish", "gmail"],
        "imessage" | "whatsapp" | "apple_calls" | "whatsapp_calls" | "gmail" => {
            vec!["scoring", "suggestions"]
        }
        _ => Vec::new(),
    }
}
