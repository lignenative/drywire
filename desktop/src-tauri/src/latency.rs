//! The latency shown for each person: an estimate, always labelled as one
//! (spec section 11, "Latency lab in the app").
//!
//! Mouth to ear, from them to this device, is their capture, one frame of
//! audio, the network from them to the relay and from the relay to here,
//! this device's jitter buffer, and its playback. This device knows its
//! own half of the path; the other half is assumed to be the same:
//!
//! - their capture plus our playback ≈ our capture plus our playback, the
//!   device latency: measured by the loopback test when it was run, else
//!   what the system reports for our two streams;
//! - their half round trip plus ours ≈ our round trip to the relay.
//!
//! Neither assumption holds for a phone talking to a PC, which is why it
//! stays an estimate; the physical method of section 11 measures the
//! truth.

/// The estimated mouth-to-ear latency from another person to this
/// device, in milliseconds.
pub fn estimate_ms(device_ms: f64, frame_ms: f64, rtt_ms: f64, buffer_ms: f64) -> f64 {
    device_ms + frame_ms + rtt_ms + buffer_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parts_add_up() {
        // A wired headset (10 ms both ways), 5 ms frames, 14 ms to the
        // relay and back, a 5 ms jitter buffer.
        assert_eq!(estimate_ms(10.0, 5.0, 14.0, 5.0), 34.0);
    }
}
