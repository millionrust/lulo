//! Mail domain types and RFC 5256/JWZ-style reference threading.
//! The result keeps placeholder ancestors so late-arriving messages can join a thread.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageHeader {
    pub message_id: Option<String>,
    /// Message IDs in wire order, oldest ancestor first.
    pub references: Vec<String>,
    pub in_reply_to: Option<String>,
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadNode {
    pub message_index: Option<usize>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadForest {
    pub nodes: Vec<ThreadNode>,
    pub roots: Vec<usize>,
}

/// Build reference chains, retain missing ancestors, prune empty containers,
/// then join root conversations with the same base subject.
pub fn thread_messages(headers: &[MessageHeader]) -> ThreadForest {
    let mut nodes = Vec::<ThreadNode>::new();
    let mut by_id = HashMap::<String, usize>::new();
    let mut message_nodes = Vec::with_capacity(headers.len());

    for (index, header) in headers.iter().enumerate() {
        let id = header.message_id.as_deref().and_then(normalize_id);
        let node = if let Some(existing) = id.as_ref().and_then(|id| by_id.get(id)).copied() {
            if nodes[existing].message_index.is_none() {
                nodes[existing].message_index = Some(index);
                existing
            } else {
                // Duplicate Message-ID headers are real input. Keep both messages,
                // but only the first can be a reference target.
                push_node(&mut nodes, Some(index))
            }
        } else {
            let node = push_node(&mut nodes, Some(index));
            if let Some(id) = id {
                by_id.insert(id, node);
            }
            node
        };
        message_nodes.push(node);
    }

    for (index, header) in headers.iter().enumerate() {
        let mut previous = None;
        let references = header.references.iter().map(String::as_str).chain(
            header
                .in_reply_to
                .as_deref()
                .filter(|_| header.references.is_empty()),
        );
        for reference in references {
            let Some(id) = normalize_id(reference) else {
                continue;
            };
            let node = *by_id
                .entry(id)
                .or_insert_with(|| push_node(&mut nodes, None));
            if let Some(parent) = previous {
                link(&mut nodes, parent, node);
            }
            previous = Some(node);
        }
        if let Some(parent) = previous {
            link(&mut nodes, parent, message_nodes[index]);
        }
    }

    // Prune empty single-child containers without recursion or moving indices.
    for node in (0..nodes.len()).rev() {
        if nodes[node].message_index.is_some() || nodes[node].children.len() != 1 {
            continue;
        }
        let child = nodes[node].children[0];
        if let Some(parent) = nodes[node].parent {
            if let Some(slot) = nodes[parent]
                .children
                .iter_mut()
                .find(|entry| **entry == node)
            {
                *slot = child;
            }
        }
        nodes[child].parent = nodes[node].parent;
        nodes[node].children.clear();
    }

    let mut roots: Vec<usize> = (0..nodes.len())
        .filter(|&node| {
            nodes[node].parent.is_none()
                && (nodes[node].message_index.is_some() || !nodes[node].children.is_empty())
        })
        .collect();

    // JWZ subject gathering happens only after references: explicit ancestry wins.
    let mut subjects = HashMap::<String, usize>::new();
    for root in roots.clone() {
        let Some(message_index) = first_message(root, &nodes) else {
            continue;
        };
        let subject = base_subject(&headers[message_index].subject);
        if subject.is_empty() {
            continue;
        }
        if let Some(&other_root) = subjects.get(&subject) {
            let container = if nodes[other_root].message_index.is_none() {
                other_root
            } else {
                let container = push_node(&mut nodes, None);
                nodes[other_root].parent = Some(container);
                nodes[container].children.push(other_root);
                subjects.insert(subject, container);
                container
            };
            nodes[root].parent = Some(container);
            nodes[container].children.push(root);
        } else {
            subjects.insert(subject, root);
        }
    }
    roots = (0..nodes.len())
        .filter(|&node| {
            nodes[node].parent.is_none()
                && (nodes[node].message_index.is_some() || !nodes[node].children.is_empty())
        })
        .collect();
    ThreadForest { nodes, roots }
}

fn push_node(nodes: &mut Vec<ThreadNode>, message_index: Option<usize>) -> usize {
    let index = nodes.len();
    nodes.push(ThreadNode {
        message_index,
        parent: None,
        children: Vec::new(),
    });
    index
}

fn link(nodes: &mut [ThreadNode], parent: usize, child: usize) {
    if parent == child || nodes[child].parent.is_some() {
        return;
    }
    let mut ancestor = Some(parent);
    while let Some(index) = ancestor {
        if index == child {
            return;
        }
        ancestor = nodes[index].parent;
    }
    nodes[child].parent = Some(parent);
    nodes[parent].children.push(child);
}

fn first_message(root: usize, nodes: &[ThreadNode]) -> Option<usize> {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if let Some(index) = nodes[node].message_index {
            return Some(index);
        }
        pending.extend(nodes[node].children.iter().rev().copied());
    }
    None
}

fn normalize_id(id: &str) -> Option<String> {
    let trimmed = id
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn base_subject(subject: &str) -> String {
    let mut subject = subject.trim();
    loop {
        let lower = subject.to_ascii_lowercase();
        let prefix = ["re:", "fw:", "fwd:"]
            .into_iter()
            .find(|prefix| lower.starts_with(prefix));
        match prefix {
            Some(prefix) => subject = subject[prefix.len()..].trim_start(),
            None => return subject.to_lowercase(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(id: &str, refs: &[&str], subject: &str) -> MessageHeader {
        MessageHeader {
            message_id: Some(id.into()),
            references: refs.iter().map(|s| (*s).into()).collect(),
            in_reply_to: None,
            subject: subject.into(),
        }
    }

    #[test]
    fn references_override_subject_and_out_of_order_messages() {
        let forest = thread_messages(&[
            header("<child>", &["<parent>"], "Different"),
            header("<parent>", &[], "Original"),
        ]);
        assert_eq!(forest.roots.len(), 1);
        let parent = forest.roots[0];
        assert_eq!(forest.nodes[parent].message_index, Some(1));
        assert_eq!(
            forest.nodes[forest.nodes[parent].children[0]].message_index,
            Some(0)
        );
    }

    #[test]
    fn missing_ancestor_with_multiple_children_is_retained() {
        let forest = thread_messages(&[
            header("a", &["missing"], "A"),
            header("b", &["missing"], "B"),
        ]);
        assert_eq!(forest.roots.len(), 1);
        assert_eq!(forest.nodes[forest.roots[0]].message_index, None);
        assert_eq!(forest.nodes[forest.roots[0]].children.len(), 2);
    }

    #[test]
    fn empty_ancestor_with_one_child_is_pruned() {
        let forest = thread_messages(&[header("a", &["missing"], "A")]);
        assert_eq!(forest.nodes[forest.roots[0]].message_index, Some(0));
    }

    #[test]
    fn subject_gathers_unreferenced_replies() {
        let forest = thread_messages(&[header("a", &[], "Hello"), header("b", &[], "Re: Hello")]);
        assert_eq!(forest.roots.len(), 1);
        assert_eq!(forest.nodes[forest.roots[0]].children.len(), 2);
    }

    #[test]
    fn cycles_and_duplicate_ids_do_not_lose_messages() {
        let forest = thread_messages(&[
            header("a", &["b"], "One"),
            header("b", &["a"], "Two"),
            header("a", &[], "Three"),
        ]);
        assert_eq!(
            forest
                .nodes
                .iter()
                .filter(|node| node.message_index.is_some())
                .count(),
            3
        );
        assert!(!forest.roots.is_empty());
    }
}
