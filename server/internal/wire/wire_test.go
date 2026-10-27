package wire

import (
	"bytes"
	"encoding/hex"
	"testing"
)

// The example header of section 3, typed by hand so that the encoding is
// pinned independently of the code that produces the test vectors.
func TestHeaderMatchesProtocolExample(t *testing.T) {
	want, _ := hex.DecodeString("01010000" + "11223344" + "00000007" + "00012c00")
	h := Header{Version: 1, Type: TypeAudio, Session: 0x11223344, Sequence: 7, Timestamp: 76800}

	if got := h.Bytes(); !bytes.Equal(got, want) {
		t.Fatalf("encoded header = %x, want %x", got, want)
	}
	if got := decodeHeader(want); got != h {
		t.Fatalf("decoded header = %+v, want %+v", got, h)
	}
}

func TestParseErrors(t *testing.T) {
	keepalive := NewKeepalive(1)
	withByte := func(i int, v byte) []byte {
		b := bytes.Clone(keepalive)
		b[i] = v
		return b
	}
	audio := func(size int) []byte {
		b := Header{Version: Version, Type: TypeAudio, Session: 1}.Bytes()
		return append(b, make([]byte, size-HeaderSize)...)
	}

	tests := []struct {
		name   string
		packet []byte
		want   error
	}{
		{"valid keepalive", keepalive, nil},
		{"valid smallest audio", audio(MinAudioSize), nil},
		{"valid largest audio", audio(MaxPacketSize), nil},
		{"empty", nil, ErrTooShort},
		{"15 bytes", keepalive[:15], ErrTooShort},
		// Size is checked before anything else, even a bad version.
		{"10 bytes, bad version", withByte(0, 9)[:10], ErrTooShort},
		{"too large", audio(MaxPacketSize + 1), ErrTooLarge},
		{"version 0", withByte(0, 0), ErrUnsupportedVersion},
		{"version 2", withByte(0, 2), ErrUnsupportedVersion},
		{"type 0", withByte(1, 0), ErrUnknownType},
		{"type 8", withByte(1, 8), ErrUnknownType},
		{"keepalive with a payload", append(bytes.Clone(keepalive), 0), ErrBadLength},
		{"audio without payload", audio(MinAudioSize - 1), ErrBadLength},
		{"session 0", NewKeepalive(0), ErrZeroSession},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if _, err := Parse(tt.packet); err != tt.want {
				t.Fatalf("Parse() error = %v, want %v", err, tt.want)
			}
		})
	}
}

func TestHandshake(t *testing.T) {
	secret := bytes.Repeat([]byte{7}, SecretSize)
	hello := NewHello(42, 1000, secret)

	if len(hello) != HelloSize {
		t.Fatalf("hello is %d bytes, want %d", len(hello), HelloSize)
	}
	if _, err := Parse(hello); err != nil {
		t.Fatalf("Parse(hello) = %v", err)
	}
	if counter, err := VerifyHandshake(hello, secret); err != nil || counter != 1000 {
		t.Fatalf("VerifyHandshake = %d, %v; want 1000, nil", counter, err)
	}

	wrongSecret := bytes.Repeat([]byte{8}, SecretSize)
	if _, err := VerifyHandshake(hello, wrongSecret); err != ErrBadMAC {
		t.Fatalf("wrong secret: error = %v, want %v", err, ErrBadMAC)
	}

	// The type byte is signed, so a hello cannot pass for a hello_ack.
	disguised := bytes.Clone(hello)
	disguised[1] = byte(TypeHelloAck)
	if _, err := VerifyHandshake(disguised, secret); err != ErrBadMAC {
		t.Fatalf("hello relabelled as hello_ack: error = %v, want %v", err, ErrBadMAC)
	}
}

func TestPingPongSameSize(t *testing.T) {
	ping := NewPing(1, 2, 3)
	pong := NewPong(1, Pong{Number: 2, ClientTime: 3, AudioReceived: 4, AudioLost: 5})
	if len(ping) != PingSize || len(pong) != PongSize || len(pong) > len(ping) {
		t.Fatalf("ping %d bytes, pong %d bytes: a pong must never be larger", len(ping), len(pong))
	}
	if got := PingClientTime(ping); got != 3 {
		t.Fatalf("PingClientTime = %d, want 3", got)
	}
	want := Pong{Number: 2, ClientTime: 3, AudioReceived: 4, AudioLost: 5}
	if got := DecodePong(pong); got != want {
		t.Fatalf("DecodePong = %+v, want %+v", got, want)
	}
}

func TestReplayWindow(t *testing.T) {
	var w ReplayWindow
	steps := []struct {
		seq  uint32
		want Verdict
	}{
		{100, VerdictNew},
		{100, VerdictDuplicate},
		{102, VerdictNew},
		{101, VerdictNew}, // late, but inside the window
		{101, VerdictDuplicate},
		{39, VerdictNew},    // 102 − 63: the oldest slot is still inside
		{38, VerdictTooOld}, // 102 − 64: just outside
		{1000, VerdictNew},  // far ahead: the window starts over
		{102, VerdictTooOld},
		{999, VerdictNew},
	}
	for i, s := range steps {
		if got := w.Check(s.seq); got != s.want {
			t.Fatalf("step %d: Check(%d) = %v, want %v", i, s.seq, got, s.want)
		}
	}
}
