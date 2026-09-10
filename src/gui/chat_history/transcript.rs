use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use gpui::{App, IntoElement, WeakEntity, Window, div};
use gpui_component::text::{
    MarkdownExtensions, MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast,
};

use super::{
    blocks::{self, BlockId, HistoryBlock},
    view::ChatHistory,
};
use crate::gui::state::TranscriptLayoutTarget;

const BLOCK_TAG: &str = "CodexTranscriptBlock";

pub(super) type TranscriptBlockStore = Arc<RwLock<HashMap<BlockId, HistoryBlock>>>;

pub(super) struct TranscriptSnapshot {
    pub markdown: String,
    pub blocks: HashMap<BlockId, HistoryBlock>,
    pub layout_targets: HashMap<TranscriptLayoutTarget, BlockId>,
}

impl TranscriptSnapshot {
    pub fn new() -> Self {
        Self {
            markdown: String::new(),
            blocks: HashMap::new(),
            layout_targets: HashMap::new(),
        }
    }

    pub fn push_markdown(&mut self, markdown: &str) {
        if markdown.is_empty() {
            return;
        }
        self.push_separator();
        self.markdown.push_str(markdown);
    }

    pub fn push_block(&mut self, block: HistoryBlock) {
        let id = block.id();
        self.push_separator();
        self.markdown
            .push_str(&format!(r#"<{BLOCK_TAG} id="{id}" />"#));
        self.blocks.insert(id, block);
    }

    pub fn map_layout_target(&mut self, target: TranscriptLayoutTarget, id: BlockId) {
        self.layout_targets.insert(target, id);
    }

    fn push_separator(&mut self) {
        if !self.markdown.is_empty() {
            self.markdown.push_str("\n\n");
        }
    }
}

#[derive(Clone)]
pub(super) struct TranscriptPlugin {
    history: WeakEntity<ChatHistory>,
    blocks: TranscriptBlockStore,
}

impl TranscriptPlugin {
    pub fn new(history: WeakEntity<ChatHistory>, blocks: TranscriptBlockStore) -> Self {
        Self { history, blocks }
    }

    pub fn extensions(self) -> MarkdownExtensions {
        MarkdownExtensions::default().plugin(self)
    }
}

#[derive(Clone)]
struct TranscriptNode {
    id: BlockId,
}

pub(super) fn node_has_id(node: &MarkdownNode, id: &BlockId) -> bool {
    node.data::<TranscriptNode>()
        .is_some_and(|node| node.id == *id)
}

impl MarkdownPlugin for TranscriptPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "codex-transcript-block"
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Html(raw) = node else {
            return None;
        };
        if html_tag_name(&raw.value) != Some(BLOCK_TAG) {
            return None;
        }
        let id = BlockId::from_marker(html_attr(&raw.value, "id")?);
        if !self
            .blocks
            .read()
            .ok()
            .is_some_and(|blocks| blocks.contains_key(&id))
        {
            return None;
        }

        Some(
            MarkdownNode::new("codex-transcript-block", TranscriptNode { id })
                .markdown(cx.node_source(node).unwrap_or(raw.value.as_str())),
        )
    }

    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let block = node
            .data::<TranscriptNode>()
            .and_then(|node| self.blocks.read().ok()?.get(&node.id).cloned());

        block
            .map(|block| blocks::render(&self.history, block, window, cx))
            .unwrap_or_else(|| div().into_any_element())
    }
}

fn html_tag_name(value: &str) -> Option<&str> {
    value
        .trim()
        .strip_prefix('<')?
        .split([' ', '/', '>'])
        .next()
}

fn html_attr(value: &str, name: &str) -> Option<String> {
    let pattern = format!(r#"{name}=""#);
    let start = value.find(&pattern)? + pattern.len();
    let end = value[start..].find('"')?;
    Some(value[start..start + end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::chat_history::blocks::UserMessageDelivery;
    use codex_app_server_protocol::UserInput;

    fn user_content(text: &str) -> Arc<[UserInput]> {
        vec![UserInput::Text {
            text: text.to_string(),
            text_elements: Vec::new(),
        }]
        .into()
    }

    #[test]
    fn streaming_markdown_is_a_strict_source_append() {
        let mut before = TranscriptSnapshot::new();
        before.push_block(HistoryBlock::User {
            key: "user-1".into(),
            turn_id: Some("turn-1".into()),
            previous_turn_id: None,
            content: user_content("hello"),
            delivery: UserMessageDelivery::Sent,
        });
        before.push_markdown("A partial reply");

        let mut after = TranscriptSnapshot::new();
        after.push_block(HistoryBlock::User {
            key: "user-1".into(),
            turn_id: Some("turn-1".into()),
            previous_turn_id: None,
            content: user_content("hello"),
            delivery: UserMessageDelivery::Sent,
        });
        after.push_markdown("A partial reply with another chunk");

        assert_eq!(
            after.markdown.strip_prefix(&before.markdown),
            Some(" with another chunk")
        );
    }

    #[test]
    fn optimistic_user_ack_keeps_the_source_marker() {
        let mut sending = TranscriptSnapshot::new();
        sending.push_block(HistoryBlock::User {
            key: "client-user-1".into(),
            turn_id: None,
            previous_turn_id: None,
            content: user_content("hello"),
            delivery: UserMessageDelivery::Sending,
        });

        let mut acknowledged = TranscriptSnapshot::new();
        acknowledged.push_block(HistoryBlock::User {
            key: "client-user-1".into(),
            turn_id: Some("turn-1".into()),
            previous_turn_id: None,
            content: user_content("hello"),
            delivery: UserMessageDelivery::Sent,
        });

        assert_eq!(sending.markdown, acknowledged.markdown);
    }

    #[test]
    fn dynamic_block_state_does_not_change_the_source_marker() {
        let mut before = TranscriptSnapshot::new();
        before.push_block(HistoryBlock::AssistantHeader {
            key: "assistant-1".into(),
            label: "Codex is working",
        });

        let mut after = TranscriptSnapshot::new();
        after.push_block(HistoryBlock::AssistantHeader {
            key: "assistant-1".into(),
            label: "Codex",
        });

        assert_eq!(after.markdown, before.markdown);
    }

    #[test]
    fn notice_body_updates_without_changing_the_source_marker() {
        let mut before = TranscriptSnapshot::new();
        before.push_block(HistoryBlock::Notice {
            key: "bridge-error".into(),
            body: "connection failed".into(),
        });

        let mut after = TranscriptSnapshot::new();
        after.push_block(HistoryBlock::Notice {
            key: "bridge-error".into(),
            body: "connection failed again".into(),
        });

        assert_eq!(after.markdown, before.markdown);
    }

    #[test]
    fn parses_only_the_transcript_tag_and_quoted_id() {
        assert_eq!(
            html_tag_name(r#" <CodexTranscriptBlock id="block-1" /> "#),
            Some(BLOCK_TAG)
        );
        assert_eq!(
            html_attr(r#"<CodexTranscriptBlock id="block-1" />"#, "id"),
            Some("block-1".into())
        );
        assert_eq!(html_tag_name("<OtherBlock />"), Some("OtherBlock"));
        assert_eq!(html_attr("<CodexTranscriptBlock />", "id"), None);
    }
}
