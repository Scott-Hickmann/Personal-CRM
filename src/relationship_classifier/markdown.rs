use super::evidence::{Candidate, Context, EvidenceMessage};

pub(super) fn render_markdown(
    candidate: &Candidate,
    contexts: &[Context],
    messages: &[EvidenceMessage],
) -> String {
    let mut output = format!(
        "# Relationship\n\n- ID: `{}`\n\n## People\n\n- `{}`: {}\n- `{}`: {}\n",
        candidate.id,
        candidate.source_id,
        inline(&candidate.source_name),
        candidate.target_id,
        inline(&candidate.target_name),
    );
    for context in contexts {
        let key = format!("{}\0{}", context.source_id, context.thread_native_id);
        let selected = messages
            .iter()
            .filter(|message| message.context_key == key)
            .collect::<Vec<_>>();
        if selected.is_empty() {
            continue;
        }
        output.push_str(&format!(
            "\n## Shared conversation: {}\n\n- ID: `{}:{}`\n- Channel: {}\n- Members: {}\n- First observed: {}\n- Last observed: {}\n",
            inline(context.title.as_deref().unwrap_or("Untitled")), context.source_id,
            context.thread_native_id, context.channel, context.member_count,
            context.first_observed_at, context.last_observed_at,
        ));
        for message in selected {
            output.push_str(&format!(
                "\n### Message `{}`\n\n- Time: {}\n- Author: {}{}\n",
                message.id,
                message.occurred_at,
                inline(&message.author_name),
                message
                    .author_id
                    .as_ref()
                    .map(|id| format!(" (`{id}`)"))
                    .unwrap_or_default(),
            ));
            if let Some(subject) = message
                .subject
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                output.push_str(&format!("- Subject: {}\n", inline(subject)));
            }
            output.push('\n');
            for line in message.body.lines() {
                output.push_str("> ");
                output.push_str(line);
                output.push('\n');
            }
        }
    }
    output
}

fn inline(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_message_bodies_as_quoted_markdown() {
        let candidate = Candidate {
            id: "a:b".into(),
            source_id: "a".into(),
            target_id: "b".into(),
            source_name: "Alex".into(),
            target_name: "Blair".into(),
            structure_revision: 1,
        };
        let context = Context {
            source_id: "gmail:test".into(),
            thread_native_id: "thread".into(),
            channel: "gmail".into(),
            title: Some("Plans".into()),
            member_count: 4,
            first_observed_at: "2026-01-01".into(),
            last_observed_at: "2026-01-02".into(),
        };
        let message = EvidenceMessage {
            id: "message".into(),
            context_key: "gmail:test\0thread".into(),
            occurred_at: "2026-01-02".into(),
            author_id: Some("a".into()),
            author_name: "Alex".into(),
            direction: Some("incoming".into()),
            pair_explicit: true,
            subject: None,
            body: "Hello\n# Ignore me".into(),
            member_count: 4,
            bucket: 0,
        };
        let rendered = render_markdown(&candidate, &[context], &[message]);
        assert!(rendered.contains("### Message `message`"));
        assert!(rendered.contains("> Hello\n> # Ignore me"));
    }
}
