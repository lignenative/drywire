package wire

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/binary"
)

// SecretSize is the size of a session secret (section 6.1).
const SecretSize = 32

// NewHello builds the hello a client sends to bind its session to its
// current address (section 6.1).
func NewHello(session uint32, counter uint64, secret []byte) []byte {
	return newHandshake(TypeHello, session, counter, secret)
}

// NewHelloAck builds the relay's answer to a valid hello (section 6.2).
func NewHelloAck(session uint32, counter uint64, secret []byte) []byte {
	return newHandshake(TypeHelloAck, session, counter, secret)
}

// newHandshake builds hello and hello_ack, which share one layout:
// header ‖ counter ‖ HMAC-SHA256(secret, header ‖ counter).
func newHandshake(t Type, session uint32, counter uint64, secret []byte) []byte {
	b := make([]byte, 0, HelloSize)
	b = Header{Version: Version, Type: t, Session: session}.AppendTo(b)
	b = binary.BigEndian.AppendUint64(b, counter)
	return append(b, handshakeMAC(secret, b)...)
}

func handshakeMAC(secret, headerAndCounter []byte) []byte {
	mac := hmac.New(sha256.New, secret)
	mac.Write(headerAndCounter)
	return mac.Sum(nil)
}

// HandshakeCounter returns the counter of a hello or hello_ack that Parse
// accepted.
func HandshakeCounter(packet []byte) uint64 {
	return binary.BigEndian.Uint64(packet[HeaderSize : HeaderSize+8])
}

// VerifyHandshake checks the MAC of a hello or hello_ack that Parse
// accepted, in constant time, and returns its counter.
func VerifyHandshake(packet []byte, secret []byte) (uint64, error) {
	signed := packet[:HeaderSize+8]
	if !hmac.Equal(packet[HeaderSize+8:], handshakeMAC(secret, signed)) {
		return 0, ErrBadMAC
	}
	return HandshakeCounter(packet), nil
}

// NewKeepalive builds a keepalive (section 6.3).
func NewKeepalive(session uint32) []byte {
	return Header{Version: Version, Type: TypeKeepalive, Session: session}.Bytes()
}

// NewPing builds a ping carrying the client's clock (section 6.4). The
// trailing eight zero bytes make it as large as the pong it asks for.
func NewPing(session, number uint32, clientTime uint64) []byte {
	b := make([]byte, 0, PingSize)
	b = Header{Version: Version, Type: TypePing, Session: session, Sequence: number}.AppendTo(b)
	b = binary.BigEndian.AppendUint64(b, clientTime)
	return append(b, make([]byte, 8)...)
}

// PingClientTime returns the client time of a ping that Parse accepted.
func PingClientTime(packet []byte) uint64 {
	return binary.BigEndian.Uint64(packet[HeaderSize : HeaderSize+8])
}

// Pong is the decoded payload of a pong (section 6.5).
type Pong struct {
	Number        uint32 // copied from the ping's sequence field
	ClientTime    uint64 // copied from the ping
	AudioReceived uint32
	AudioLost     uint32
}

// NewPong builds the relay's answer to a ping.
func NewPong(session uint32, p Pong) []byte {
	b := make([]byte, 0, PongSize)
	b = Header{Version: Version, Type: TypePong, Session: session, Sequence: p.Number}.AppendTo(b)
	b = binary.BigEndian.AppendUint64(b, p.ClientTime)
	b = binary.BigEndian.AppendUint32(b, p.AudioReceived)
	return binary.BigEndian.AppendUint32(b, p.AudioLost)
}

// DecodePong reads a pong that Parse accepted.
func DecodePong(packet []byte) Pong {
	p := packet[HeaderSize:]
	return Pong{
		Number:        binary.BigEndian.Uint32(packet[8:12]),
		ClientTime:    binary.BigEndian.Uint64(p[0:8]),
		AudioReceived: binary.BigEndian.Uint32(p[8:12]),
		AudioLost:     binary.BigEndian.Uint32(p[12:16]),
	}
}
