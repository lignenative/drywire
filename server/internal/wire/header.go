// Package wire encodes and decodes Drywire packets as the relay sees them:
// the 16-byte header and the control packets (hello, hello_ack, keepalive,
// ping, pong). Audio payloads are encrypted end to end, so this package only
// checks their size; it never looks inside them.
//
// docs/protocol.md is the normative description of everything here. Section
// numbers in comments refer to that document.
package wire

import "encoding/binary"

// Version is the protocol version this package speaks (section 3).
const Version uint8 = 1

// SupportedVersions lists the versions the relay accepts: the current one
// and, once it exists, the previous one (section 11).
var SupportedVersions = []uint8{Version}

// Sizes from sections 2 and 4.
const (
	HeaderSize    = 16
	MaxPacketSize = 1200
	TagSize       = 16 // AES-GCM authentication tag at the end of audio packets
	MinAudioSize  = HeaderSize + 1 + TagSize
	HelloSize     = HeaderSize + 8 + 32 // header ‖ counter ‖ HMAC-SHA256
	KeepaliveSize = HeaderSize
	PingSize      = HeaderSize + 8 + 8 // header ‖ client time ‖ zero
	PongSize      = HeaderSize + 8 + 4 + 4
)

// Type is the packet type (section 4).
type Type uint8

const (
	TypeAudio     Type = 0x01
	TypeHello     Type = 0x02
	TypeHelloAck  Type = 0x03
	TypeKeepalive Type = 0x04
	TypePing      Type = 0x05
	TypePong      Type = 0x06
)

// String returns the name used in docs/protocol.md and in the vectors.
func (t Type) String() string {
	switch t {
	case TypeAudio:
		return "audio"
	case TypeHello:
		return "hello"
	case TypeHelloAck:
		return "hello_ack"
	case TypeKeepalive:
		return "keepalive"
	case TypePing:
		return "ping"
	case TypePong:
		return "pong"
	}
	return "unknown"
}

// Flags of an audio packet (section 5).
const (
	FlagRedundant  uint8 = 0x01
	FlagEndOfSpurt uint8 = 0x02
)

// Header is the decoded form of the 16 bytes that start every packet.
type Header struct {
	Version   uint8
	Type      Type
	Flags     uint8
	KeyEpoch  uint8
	Session   uint32
	Sequence  uint32
	Timestamp uint32
}

// AppendTo appends the 16 encoded bytes of h to b and returns the result.
func (h Header) AppendTo(b []byte) []byte {
	b = append(b, h.Version, uint8(h.Type), h.Flags, h.KeyEpoch)
	b = binary.BigEndian.AppendUint32(b, h.Session)
	b = binary.BigEndian.AppendUint32(b, h.Sequence)
	b = binary.BigEndian.AppendUint32(b, h.Timestamp)
	return b
}

// Bytes returns the 16 encoded bytes of h.
func (h Header) Bytes() []byte {
	return h.AppendTo(make([]byte, 0, HeaderSize))
}

// decodeHeader reads the first 16 bytes of b, which must be long enough.
func decodeHeader(b []byte) Header {
	return Header{
		Version:   b[0],
		Type:      Type(b[1]),
		Flags:     b[2],
		KeyEpoch:  b[3],
		Session:   binary.BigEndian.Uint32(b[4:8]),
		Sequence:  binary.BigEndian.Uint32(b[8:12]),
		Timestamp: binary.BigEndian.Uint32(b[12:16]),
	}
}
