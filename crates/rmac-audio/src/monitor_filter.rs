//! Decides which `pw-dump --monitor` updates can change audio state.
//!
//! Every `pw-dump` run, including the one each [`crate::snapshot`] makes,
//! connects to PipeWire as a client, and the monitor reports that client
//! arriving, updating its props and leaving. Treating those as changes made
//! every watcher re-read the graph, which started another `pw-dump`, which
//! the monitor reported again: a self-sustaining loop of about one snapshot
//! a second in every process that watches audio. Only the objects a
//! snapshot reads (nodes, devices and metadata) count as changes here.

use std::collections::HashMap;

/// Tracks the type of every object the monitor has described, so that a
/// removal (which carries only an id) can be classified too.
#[derive(Debug, Default)]
pub(crate) struct MonitorFilter {
    relevant: HashMap<u64, bool>,
    seen_initial_graph: bool,
}

impl MonitorFilter {
    /// Absorb one JSON array printed by the monitor. Returns whether it
    /// describes a change a snapshot could observe. The first array is the
    /// full graph and always counts.
    pub(crate) fn absorb(&mut self, update: &serde_json::Value) -> bool {
        let initial = !self.seen_initial_graph;
        self.seen_initial_graph = true;
        let Some(objects) = update.as_array() else {
            // Not the documented framing: re-read rather than miss a change.
            return true;
        };
        let mut changed = initial;
        for object in objects {
            let Some(id) = object.get("id").and_then(serde_json::Value::as_u64) else {
                changed = true;
                continue;
            };
            let removed = object.get("info").is_none_or(serde_json::Value::is_null)
                && object.get("type").is_none();
            if removed {
                // An id the monitor never typed is treated as relevant.
                changed |= self.relevant.remove(&id).unwrap_or(true);
                continue;
            }
            let relevant = object
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_none_or(type_can_change_audio_state);
            self.relevant.insert(id, relevant);
            changed |= relevant;
        }
        changed
    }
}

fn type_can_change_audio_state(kind: &str) -> bool {
    let kind = kind.rsplit(':').next().unwrap_or(kind);
    matches!(kind, "Node" | "Device" | "Metadata")
}

/// Split every complete JSON value off the front of `buffer`, leaving a
/// trailing partial value for the next read.
pub(crate) fn drain_json_values(
    buffer: &mut Vec<u8>,
) -> Result<Vec<serde_json::Value>, serde_json::Error> {
    let mut values = Vec::new();
    let mut stream = serde_json::Deserializer::from_slice(buffer).into_iter::<serde_json::Value>();
    let mut consumed = 0;
    loop {
        match stream.next() {
            Some(Ok(value)) => {
                consumed = stream.byte_offset();
                values.push(value);
            }
            Some(Err(error)) if error.is_eof() => break,
            Some(Err(error)) => return Err(error),
            None => {
                consumed = stream.byte_offset();
                break;
            }
        }
    }
    buffer.drain(..consumed);
    Ok(values)
}

/// Reads `pw-dump --monitor` output as it arrives and says whether it holds
/// a change an audio snapshot could observe. Shared by every process that
/// watches audio through its own monitor, such as the menu bar's status
/// watcher.
#[derive(Debug, Default)]
pub struct MonitorChanges {
    pending: Vec<u8>,
    filter: MonitorFilter,
}

impl MonitorChanges {
    /// Feed the next bytes read from the monitor's stdout. Returns whether
    /// any complete update among them can change audio state; a partial
    /// update waits for the next read.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<bool, String> {
        const PENDING_LIMIT: usize = 16 * 1024 * 1024;
        self.pending.extend_from_slice(bytes);
        if self.pending.len() > PENDING_LIMIT {
            return Err("pw-dump --monitor sent an update larger than 16 MiB".to_owned());
        }
        let values = drain_json_values(&mut self.pending).map_err(|error| error.to_string())?;
        Ok(values
            .iter()
            .fold(false, |changed, value| self.filter.absorb(value) | changed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_changes_ignore_a_snapshot_client_split_across_reads() {
        let mut changes = MonitorChanges::default();
        assert_eq!(
            changes.feed(b"[{\"id\":40,\"type\":\"PipeWire:Interface:Node\",\"info\":{}}]\n"),
            Ok(true)
        );
        let client = b"[{\"id\":67,\"type\":\"PipeWire:Interface:Client\",\"info\":{}}]\n";
        assert_eq!(changes.feed(&client[..20]), Ok(false));
        assert_eq!(changes.feed(&client[20..]), Ok(false));
        assert_eq!(changes.feed(b"[{\"id\":67,\"info\":null}]\n"), Ok(false));
        assert_eq!(
            changes.feed(b"[{\"id\":40,\"type\":\"PipeWire:Interface:Node\",\"info\":{}}]"),
            Ok(true)
        );
    }

    fn object(id: u64, kind: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "type": format!("PipeWire:Interface:{kind}"),
            "info": { "props": {} }
        })
    }

    fn removal(id: u64) -> serde_json::Value {
        serde_json::json!({ "id": id, "info": null })
    }

    #[test]
    fn a_pw_dump_client_coming_and_going_is_not_an_audio_change() {
        let mut filter = MonitorFilter::default();
        assert!(filter.absorb(&serde_json::json!([
            object(40, "Node"),
            object(41, "Client")
        ])));

        // What another process's snapshot looks like to the monitor.
        assert!(!filter.absorb(&serde_json::json!([object(67, "Client")])));
        assert!(!filter.absorb(&serde_json::json!([removal(67)])));
        assert!(!filter.absorb(&serde_json::json!([object(41, "Client")])));
    }

    #[test]
    fn nodes_devices_metadata_and_unknown_removals_are_changes() {
        let mut filter = MonitorFilter::default();
        assert!(filter.absorb(&serde_json::json!([])));
        assert!(filter.absorb(&serde_json::json!([object(40, "Node")])));
        assert!(filter.absorb(&serde_json::json!([object(50, "Device")])));
        assert!(filter.absorb(&serde_json::json!([object(60, "Metadata")])));
        assert!(filter.absorb(&serde_json::json!([removal(40)])));
        assert!(filter.absorb(&serde_json::json!([removal(999)])));
        assert!(!filter.absorb(&serde_json::json!([object(70, "Port")])));
        assert!(!filter.absorb(&serde_json::json!([removal(70)])));
    }

    #[test]
    fn json_values_are_split_across_reads() {
        let mut buffer = b"[{\"id\":1}]\n[{\"id\"".to_vec();
        let values = drain_json_values(&mut buffer).unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(buffer, b"\n[{\"id\"".to_vec());
        buffer.extend_from_slice(b":2}]\n");
        let values = drain_json_values(&mut buffer).unwrap();
        assert_eq!(values, vec![serde_json::json!([{ "id": 2 }])]);
        assert!(buffer.iter().all(u8::is_ascii_whitespace));
        assert!(drain_json_values(&mut b"]".to_vec()).is_err());
    }
}
