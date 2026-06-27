//! Wire codecs for [`A2aMessage`].

use crate::{A2aError, A2aCodec, A2aMessage, AgentId, ConversationId, MessageId};

/// JSON codec — the interop form for non-UNL agents. Serializes the whole
/// message (including the content graph) as JSON.
#[derive(Default)]
pub struct JsonCodec;

impl A2aCodec for JsonCodec {
    fn encode(&self, msg: &A2aMessage) -> Vec<u8> {
        serde_json::to_vec(msg).expect("A2aMessage serializes")
    }

    fn decode(&self, bytes: &[u8]) -> Result<A2aMessage, A2aError> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

/// The compact default wire form: a small `key: value` header, a `---`
/// separator, then the content graph in UNL list format. Small enough for
/// constrained transports when UCLs (numeric) are used.
///
/// ```text
/// sender: alice
/// receiver: bob
/// conversation: c-42
/// gloss: Peter killed John
/// ---
/// [W]
/// 01: kill.@past.@entry
/// ...
/// [/W]
/// [R]
/// agt(01, 02)
/// [/R]
/// ```
#[derive(Default)]
pub struct UnlWireCodec;

impl A2aCodec for UnlWireCodec {
    fn encode(&self, msg: &A2aMessage) -> Vec<u8> {
        let mut out = String::new();
        out.push_str("sender: ");
        out.push_str(&msg.sender.0);
        out.push('\n');
        out.push_str("receiver: ");
        out.push_str(&msg.receiver.0);
        out.push('\n');
        out.push_str("conversation: ");
        out.push_str(&msg.conversation_id.0);
        out.push('\n');
        if let Some(r) = &msg.reply_to {
            out.push_str("reply-to: ");
            out.push_str(&r.0);
            out.push('\n');
        }
        if let Some(g) = &msg.gloss {
            // Single-line gloss (newlines would break the header grammar).
            out.push_str("gloss: ");
            out.push_str(&g.replace('\n', " "));
            out.push('\n');
        }
        out.push_str("---\n");
        out.push_str(&unl_parser::serialize_list(&msg.content));
        out.into_bytes()
    }

    fn decode(&self, bytes: &[u8]) -> Result<A2aMessage, A2aError> {
        let text = std::str::from_utf8(bytes).map_err(|_| A2aError::Utf8)?;
        let (header, content) = text
            .split_once("\n---\n")
            .ok_or_else(|| A2aError::Malformed("missing '---' separator".into()))?;

        let mut sender = None;
        let mut receiver = None;
        let mut conversation = None;
        let mut reply_to = None;
        let mut gloss = None;
        for line in header.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = line
                .split_once(": ")
                .ok_or_else(|| A2aError::Malformed(format!("bad header line: {line}")))?;
            match key {
                "sender" => sender = Some(AgentId::from(value)),
                "receiver" => receiver = Some(AgentId::from(value)),
                "conversation" => conversation = Some(ConversationId::from(value)),
                "reply-to" => reply_to = Some(MessageId::from(value)),
                "gloss" => gloss = Some(value.to_string()),
                other => return Err(A2aError::Malformed(format!("unknown header: {other}"))),
            }
        }

        let graph = unl_parser::parse_list(content)?;
        Ok(A2aMessage {
            sender: sender.ok_or_else(|| A2aError::Malformed("missing sender".into()))?,
            receiver: receiver.ok_or_else(|| A2aError::Malformed("missing receiver".into()))?,
            conversation_id: conversation
                .ok_or_else(|| A2aError::Malformed("missing conversation".into()))?,
            content: graph,
            gloss,
            reply_to,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unl_core::{Relation, RelationTag, Uci, UnlGraph, Uw};

    /// "Peter killed John" in list-canonical form.
    fn sample_graph() -> UnlGraph {
        let mut g = UnlGraph::new();
        g.insert_node("01", Uw::new(Uci::ucn("kill")));
        g.insert_node("02", Uw::new(Uci::ucn("Peter")));
        g.insert_node("03", Uw::new(Uci::ucn("John")));
        g.entry = Some("01".into());
        g.add_relation(Relation::between(RelationTag::Agt, "01".into(), "02".into()));
        g.add_relation(Relation::between(RelationTag::Obj, "01".into(), "03".into()));
        g
    }

    fn sample_message() -> A2aMessage {
        A2aMessage {
            sender: "alice".into(),
            receiver: "bob".into(),
            conversation_id: "c-42".into(),
            content: sample_graph(),
            gloss: Some("Peter killed John".into()),
            reply_to: Some("m-7".into()),
        }
    }

    #[test]
    fn json_roundtrip() {
        let msg = sample_message();
        let bytes = JsonCodec.encode(&msg);
        assert_eq!(JsonCodec.decode(&bytes).unwrap(), msg);
    }

    #[test]
    fn unl_wire_roundtrip() {
        let msg = sample_message();
        let bytes = UnlWireCodec.encode(&msg);
        assert_eq!(UnlWireCodec.decode(&bytes).unwrap(), msg);
    }

    #[test]
    fn unl_wire_roundtrip_minimal() {
        // No gloss, no reply-to.
        let msg = A2aMessage::new("alice", "bob", "c-1", sample_graph());
        let bytes = UnlWireCodec.encode(&msg);
        assert_eq!(UnlWireCodec.decode(&bytes).unwrap(), msg);
    }

    #[test]
    fn unl_wire_is_compact() {
        // The semantic content is meaning, not prose: a short list-format body.
        let msg = sample_message();
        let bytes = UnlWireCodec.encode(&msg);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("agt(01, 02)"));
        assert!(text.starts_with("sender: alice"));
    }

    #[test]
    fn decode_rejects_missing_separator() {
        let err = UnlWireCodec.decode(b"sender: a\nreceiver: b\n").unwrap_err();
        assert!(matches!(err, A2aError::Malformed(_)));
    }

    #[test]
    fn decode_rejects_missing_field() {
        let wire = b"sender: alice\nconversation: c\n---\n[W]\n[/W]\n[R]\n[/R]\n";
        let err = UnlWireCodec.decode(wire).unwrap_err();
        assert!(matches!(err, A2aError::Malformed(_))); // missing receiver
    }
}
