//! Reading a Server-Sent Events stream: `event:` and `data:` lines, an
//! empty line to end each event, `:` for comments (the server's
//! keepalives).

use std::io::{BufRead, BufReader, Read};

use serde::Deserialize;

use crate::{Error, Event, Presence};

/// A space's events, in order. Iterating blocks until the next one; the
/// iterator ends when the server closes the stream (the caller left the
/// space, fell behind, or the server stopped): open a new one to resume
/// from a fresh snapshot.
pub struct Events {
    reader: BufReader<Box<dyn Read + Send>>,
}

impl Events {
    pub(crate) fn new(reader: Box<dyn Read + Send>) -> Events {
        Events {
            reader: BufReader::new(reader),
        }
    }
}

#[derive(Deserialize)]
struct Snapshot {
    sessions: Vec<Presence>,
}

/// Turns one event's name and data into an [`Event`].
fn decode(name: &str, data: &str) -> Result<Event, Error> {
    let bad = |e: serde_json::Error| Error::Protocol(format!("{name} event: {e}"));
    Ok(match name {
        "snapshot" => Event::Snapshot(
            serde_json::from_str::<Snapshot>(data)
                .map_err(bad)?
                .sessions,
        ),
        "join" => Event::Join(serde_json::from_str(data).map_err(bad)?),
        "leave" => Event::Leave(serde_json::from_str(data).map_err(bad)?),
        "disconnect" => Event::Disconnect(serde_json::from_str(data).map_err(bad)?),
        other => Event::Other(other.to_string()),
    })
}

impl Iterator for Events {
    type Item = Result<Event, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut name = String::new();
        let mut data = String::new();
        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e) => return Some(Err(Error::Network(e.to_string()))),
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if name.is_empty() && data.is_empty() {
                    continue; // after a comment
                }
                let event = if name.is_empty() { "message" } else { &name };
                return Some(decode(event, &data));
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                "event" => name = value.to_string(),
                "data" => {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(value);
                }
                _ => {} // id, retry: not used by this protocol
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hex;

    #[test]
    fn a_stream_is_read_event_by_event() {
        let text = ": keepalive\n\n\
            event: snapshot\ndata: {\"sessions\":[]}\n\n\
            event: join\ndata: {\"session\":7,\"room_id\":\"0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f\",\
            \"public_key\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"name\":\"sam\"}\r\n\r\n\
            event: renamed\ndata: {}\n\n";
        let mut events = Events::new(Box::new(std::io::Cursor::new(text.as_bytes().to_vec())));
        assert_eq!(events.next().unwrap().unwrap(), Event::Snapshot(vec![]));
        assert_eq!(
            events.next().unwrap().unwrap(),
            Event::Join(Presence {
                session: 7,
                room_id: Hex([0x0f; 16]),
                public_key: Hex([0xaa; 32]),
                name: "sam".into(),
            })
        );
        assert_eq!(
            events.next().unwrap().unwrap(),
            Event::Other("renamed".into())
        );
        assert!(events.next().is_none());
    }
}
