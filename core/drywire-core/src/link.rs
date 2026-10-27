//! The client's side of its session with the relay: `hello` until the relay
//! confirms the binding, a `ping` every second, a `keepalive` when nothing
//! else was sent for ten seconds (protocol section 6).
//!
//! [`Link`] is a state machine. The caller tells it the time and what came
//! in, and asks it what to send; it never touches a socket or a clock, so
//! tests run it in simulated time.

use drywire_proto::control::{
    Pong, SECRET_SIZE, decode_pong, new_hello, new_keepalive, new_ping, verify_handshake,
};
use drywire_proto::{Error, PacketType, parse};

/// How often a `hello` is repeated until it is answered.
pub const HELLO_RETRY_US: u64 = 250_000;
/// A client sends a `keepalive` when it sent nothing else for this long.
pub const KEEPALIVE_US: u64 = 10_000_000;
/// Without a `pong` for this long, the binding is presumed lost (the relay
/// restarted, or the NAT forgot us) and the client says `hello` again.
pub const PONG_TIMEOUT_US: u64 = 5_000_000;

/// When a [`Link`] pings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkConfig {
    /// Time between two pings, or `None` for no pings (then keepalives keep
    /// the binding).
    pub ping_interval_us: Option<u64>,
}

impl Default for LinkConfig {
    fn default() -> LinkConfig {
        LinkConfig {
            ping_interval_us: Some(1_000_000),
        }
    }
}

/// What the relay told us, and what we sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    /// Whether the relay confirmed our binding and still answers.
    pub bound: bool,
    /// `hello` packets sent.
    pub hellos: u64,
    /// Bindings confirmed by a `hello_ack`.
    pub bindings: u64,
    /// `ping` packets sent.
    pub pings: u64,
    /// `pong` packets received.
    pub pongs: u64,
    /// `keepalive` packets sent.
    pub keepalives: u64,
    /// Round trip of the last pong, in microseconds.
    pub rtt_us: Option<u64>,
    /// Smallest round trip seen, in microseconds.
    pub min_rtt_us: Option<u64>,
    /// Our audio packets the relay received since the binding (from the
    /// last pong).
    pub uplink_received: u32,
    /// Our audio packets the relay expected but never received.
    pub uplink_lost: u32,
}

/// The state of one client's session with the relay.
pub struct Link {
    session: u32,
    secret: [u8; SECRET_SIZE],
    config: LinkConfig,
    /// Counter of the last `hello` sent, and when.
    hello: Option<(u64, u64)>,
    next_ping_us: u64,
    ping_number: u32,
    /// Last time the relay showed it knows us: a `hello_ack` or a `pong`.
    heard_us: u64,
    /// Last time this link sent anything.
    sent_us: u64,
    stats: LinkStats,
}

impl Link {
    /// A link that is not bound yet.
    pub fn new(session: u32, secret: [u8; SECRET_SIZE], config: LinkConfig) -> Link {
        Link {
            session,
            secret,
            config,
            hello: None,
            next_ping_us: 0,
            ping_number: 0,
            heard_us: 0,
            sent_us: 0,
            stats: LinkStats::default(),
        }
    }

    /// The counters so far.
    pub fn stats(&self) -> LinkStats {
        self.stats
    }

    /// Whether the relay confirmed our binding.
    pub fn is_bound(&self) -> bool {
        self.stats.bound
    }

    /// The packet to send now, if one is due. `unix_ms` feeds the `hello`
    /// counter, which must grow across restarts; `other_sent_us` is the last
    /// time anything else (audio, an announcement) was sent, which makes a
    /// keepalive unnecessary.
    pub fn poll(&mut self, now_us: u64, unix_ms: u64, other_sent_us: u64) -> Option<Vec<u8>> {
        if self.stats.bound
            && self.config.ping_interval_us.is_some()
            && now_us.saturating_sub(self.heard_us) >= PONG_TIMEOUT_US
        {
            self.stats.bound = false;
        }
        if !self.stats.bound {
            let due = self
                .hello
                .is_none_or(|(_, at)| now_us.saturating_sub(at) >= HELLO_RETRY_US);
            if !due {
                return None;
            }
            // A new counter for every attempt (section 6.1), even within one
            // millisecond.
            let counter = self.hello.map_or(unix_ms, |(c, _)| unix_ms.max(c + 1));
            self.hello = Some((counter, now_us));
            self.stats.hellos += 1;
            return Some(self.sent(
                now_us,
                new_hello(self.session, counter, &self.secret).to_vec(),
            ));
        }
        if let Some(interval) = self.config.ping_interval_us
            && now_us >= self.next_ping_us
        {
            self.next_ping_us = now_us + interval;
            self.ping_number = self.ping_number.wrapping_add(1);
            self.stats.pings += 1;
            let ping = new_ping(self.session, self.ping_number, now_us);
            return Some(self.sent(now_us, ping.to_vec()));
        }
        if now_us.saturating_sub(self.sent_us.max(other_sent_us)) >= KEEPALIVE_US {
            self.stats.keepalives += 1;
            return Some(self.sent(now_us, new_keepalive(self.session).to_vec()));
        }
        None
    }

    fn sent(&mut self, now_us: u64, packet: Vec<u8>) -> Vec<u8> {
        self.sent_us = now_us;
        packet
    }

    /// Handles a `hello_ack` or `pong`. Returns whether the packet was one
    /// of those and valid; other types are not the link's business.
    pub fn handle(&mut self, now_us: u64, packet: &[u8]) -> Result<bool, Error> {
        let header = parse(packet)?;
        if header.session != self.session {
            return Ok(false);
        }
        match header.packet_type {
            PacketType::HelloAck => {
                let counter = verify_handshake(packet, &self.secret)?;
                // Only the answer to our last hello counts (section 6.2).
                if self.hello.map(|(c, _)| c) != Some(counter) {
                    return Ok(false);
                }
                if !self.stats.bound {
                    self.stats.bound = true;
                    self.stats.bindings += 1;
                    self.next_ping_us = now_us;
                }
                self.heard_us = now_us;
                Ok(true)
            }
            PacketType::Pong => {
                let Pong {
                    client_time,
                    audio_received,
                    audio_lost,
                    ..
                } = decode_pong(packet);
                // The relay copies our clock back; a value from the future
                // is not ours.
                if client_time > now_us {
                    return Ok(false);
                }
                let rtt = now_us - client_time;
                self.stats.pongs += 1;
                self.stats.rtt_us = Some(rtt);
                self.stats.min_rtt_us = Some(self.stats.min_rtt_us.map_or(rtt, |m| m.min(rtt)));
                self.stats.uplink_received = audio_received;
                self.stats.uplink_lost = audio_lost;
                self.heard_us = now_us;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// When [`Link::poll`] may next have something to send, so a network
    /// loop knows how long it can wait for packets.
    pub fn next_deadline_us(&self, other_sent_us: u64) -> u64 {
        if !self.stats.bound {
            return self.hello.map_or(0, |(_, at)| at + HELLO_RETRY_US);
        }
        let keepalive = self.sent_us.max(other_sent_us) + KEEPALIVE_US;
        match self.config.ping_interval_us {
            Some(_) => self.next_ping_us.min(keepalive),
            None => keepalive,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drywire_proto::control::{new_hello_ack, new_pong};

    const SECRET: [u8; 32] = [5; 32];

    fn kind(packet: &[u8]) -> PacketType {
        parse(packet).unwrap().packet_type
    }

    /// Plays the relay's part: answers a hello.
    fn ack(hello: &[u8]) -> Vec<u8> {
        let counter = verify_handshake(hello, &SECRET).unwrap();
        new_hello_ack(1, counter, &SECRET).to_vec()
    }

    #[test]
    fn hello_repeats_with_a_new_counter_until_acknowledged() {
        let mut link = Link::new(1, SECRET, LinkConfig::default());
        let first = link.poll(0, 1000, 0).unwrap();
        assert_eq!(kind(&first), PacketType::Hello);
        assert_eq!(link.poll(100_000, 1000, 0), None, "too soon to retry");
        let second = link.poll(HELLO_RETRY_US, 1000, 0).unwrap();
        assert_eq!(verify_handshake(&second, &SECRET), Ok(1001));

        // The answer to the first hello is stale.
        assert_eq!(link.handle(300_000, &ack(&first)), Ok(false));
        assert!(!link.is_bound());
        assert_eq!(link.handle(300_000, &ack(&second)), Ok(true));
        assert!(link.is_bound());
        // A forged ack does not verify.
        let forged = new_hello_ack(1, 1001, &[6; 32]);
        assert_eq!(link.handle(300_000, &forged), Err(Error::BadMac));
    }

    #[test]
    fn pings_every_second_and_reads_the_round_trip() {
        let mut link = Link::new(1, SECRET, LinkConfig::default());
        let hello = link.poll(0, 1, 0).unwrap();
        link.handle(1_000, &ack(&hello)).unwrap();

        let ping = link.poll(1_000, 1, 0).unwrap();
        assert_eq!(kind(&ping), PacketType::Ping);
        assert_eq!(link.poll(500_000, 1, 0), None);
        let pong = new_pong(
            1,
            Pong {
                number: 1,
                client_time: 1_000,
                audio_received: 10,
                audio_lost: 2,
            },
        );
        assert_eq!(link.handle(3_500, &pong), Ok(true));
        let stats = link.stats();
        assert_eq!(stats.rtt_us, Some(2_500));
        assert_eq!((stats.uplink_received, stats.uplink_lost), (10, 2));
        assert_eq!(kind(&link.poll(1_001_000, 1, 0).unwrap()), PacketType::Ping);
        assert_eq!(link.next_deadline_us(0), 2_001_000);
    }

    #[test]
    fn keepalive_only_after_ten_silent_seconds() {
        let config = LinkConfig {
            ping_interval_us: None,
        };
        let mut link = Link::new(1, SECRET, config);
        let hello = link.poll(0, 1, 0).unwrap();
        link.handle(0, &ack(&hello)).unwrap();
        // Audio sent at 4 s pushes the keepalive back.
        assert_eq!(link.poll(KEEPALIVE_US, 1, 4_000_000), None);
        assert_eq!(link.next_deadline_us(4_000_000), 14_000_000);
        let packet = link.poll(14_000_000, 1, 4_000_000).unwrap();
        assert_eq!(kind(&packet), PacketType::Keepalive);
        assert_eq!(link.stats().keepalives, 1);
    }

    #[test]
    fn says_hello_again_when_the_relay_stops_answering() {
        let mut link = Link::new(1, SECRET, LinkConfig::default());
        let hello = link.poll(0, 1, 0).unwrap();
        link.handle(0, &ack(&hello)).unwrap();
        let mut now = 0;
        while now < PONG_TIMEOUT_US {
            let packet = link.poll(now, 1, 0);
            assert!(packet.is_none_or(|p| kind(&p) == PacketType::Ping));
            now += 100_000;
        }
        let again = link.poll(now, 2, 0).unwrap();
        assert_eq!(kind(&again), PacketType::Hello);
        assert!(!link.is_bound());
    }
}
