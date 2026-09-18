//! Shared stream-event application: the single generic mechanism by which
//! incremental backend events become transcript blocks.
//!
//! Both [`MockBackend`](super::mock::MockBackend) and the OpenCode adapter
//! route their `StreamEvent` queues through [`apply_event`]; there is exactly
//! one streaming architecture (§8 of the Phase-4 brief).

use super::{Block, StreamEvent};

/// Apply one stream event to a session's block list, in order.
pub fn apply_event(blocks: &mut Vec<Block>, event: &StreamEvent) {
    match event {
        StreamEvent::Chunk(text) => match blocks.last_mut() {
            Some(Block::Assistant { text: existing }) => existing.push_str(text),
            _ => blocks.push(Block::Assistant { text: text.clone() }),
        },
        StreamEvent::ThinkChunk(text) => match blocks.last_mut() {
            Some(Block::Thinking { text: existing }) => existing.push_str(text),
            _ => blocks.push(Block::Thinking { text: text.clone() }),
        },
        StreamEvent::Push(block) => match (&block, blocks.last()) {
            // Consecutive working indicators collapse into one (multi-step
            // turns emit a spinner per step; only the latest label matters).
            (Block::Working { label }, Some(Block::Working { .. })) => {
                if let Some(Block::Working { label: existing }) = blocks.last_mut() {
                    *existing = label.clone();
                }
            }
            _ => blocks.push(block.clone()),
        },
        StreamEvent::UpdateTool {
            state,
            elapsed,
            output,
        } => {
            // Targets the latest tool/shell block even when a working
            // indicator or note was pushed after it.
            for block in blocks.iter_mut().rev() {
                match block {
                    Block::Tool { call } => {
                        call.state = state.clone();
                        call.elapsed = elapsed.clone();
                        call.output = output.clone();
                        break;
                    }
                    Block::Shell { run } => {
                        run.state = state.clone();
                        break;
                    }
                    _ => {}
                }
            }
        }
        StreamEvent::ShellLine(line) => {
            for block in blocks.iter_mut().rev() {
                if let Block::Shell { run } = block {
                    run.output.push(line.clone());
                    break;
                }
            }
        }
        StreamEvent::WorkLabel(label) => {
            // An empty label retires the spinner at stream end.
            if label.is_empty() {
                if matches!(blocks.last(), Some(Block::Working { .. })) {
                    blocks.pop();
                }
                return;
            }
            match blocks.last_mut() {
                Some(Block::Working { label: existing }) => *existing = label.clone(),
                _ => blocks.push(Block::Working {
                    label: label.clone(),
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::apply_event;
    use crate::backend::{Block, StreamEvent, ToolCall, ToolState};

    #[test]
    fn chunks_share_one_assistant_block() {
        let mut blocks = Vec::new();
        apply_event(&mut blocks, &StreamEvent::Chunk("a".into()));
        apply_event(&mut blocks, &StreamEvent::Chunk("b".into()));
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Assistant { text } => assert_eq!(text, "ab"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn think_chunks_share_one_thinking_block() {
        let mut blocks = Vec::new();
        apply_event(&mut blocks, &StreamEvent::ThinkChunk("a".into()));
        apply_event(&mut blocks, &StreamEvent::Chunk("b".into()));
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn update_tool_sets_state_and_output() {
        let mut blocks = vec![Block::Tool {
            call: ToolCall {
                name: "t".into(),
                detail: "d".into(),
                state: ToolState::Running,
                output: Vec::new(),
                elapsed: "0s".into(),
            },
        }];
        apply_event(
            &mut blocks,
            &StreamEvent::UpdateTool {
                state: ToolState::Done,
                elapsed: "1s".into(),
                output: vec!["ok".into()],
            },
        );
        match &blocks[0] {
            Block::Tool { call } => {
                assert_eq!(call.state, ToolState::Done);
                assert_eq!(call.output, vec!["ok".to_owned()]);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
