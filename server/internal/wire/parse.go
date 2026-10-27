package wire

import "slices"

// Error is one of the error codes of section 9. Codes are plain strings so
// they can be compared with == and written as-is in logs and test vectors.
type Error string

func (e Error) Error() string { return string(e) }

// Error codes, in the order the checks run (section 9).
const (
	ErrTooShort           Error = "too_short"
	ErrTooLarge           Error = "too_large"
	ErrUnsupportedVersion Error = "unsupported_version"
	ErrUnknownType        Error = "unknown_type"
	ErrBadLength          Error = "bad_length"
	ErrZeroSession        Error = "zero_session"
	ErrBadMAC             Error = "bad_mac"
	ErrBadTag             Error = "bad_tag"       // receivers only, see package clientref
	ErrBadFraming         Error = "bad_framing"   // receivers only, see package clientref
	ErrBadSignature       Error = "bad_signature" // receivers only, see package clientref
)

// Parse checks a datagram's size and header and returns the decoded header.
// It runs every check that needs no key: what comes after the header is
// verified by VerifyHandshake for hello packets, and by receivers for audio.
func Parse(packet []byte) (Header, error) {
	if len(packet) < HeaderSize {
		return Header{}, ErrTooShort
	}
	if len(packet) > MaxPacketSize {
		return Header{}, ErrTooLarge
	}
	h := decodeHeader(packet)
	if !slices.Contains(SupportedVersions, h.Version) {
		return Header{}, ErrUnsupportedVersion
	}
	if h.Type < TypeAudio || h.Type > TypeAnnounce {
		return Header{}, ErrUnknownType
	}
	if !sizeIsValid(h.Type, len(packet)) {
		return Header{}, ErrBadLength
	}
	if h.Session == 0 {
		return Header{}, ErrZeroSession
	}
	return h, nil
}

// sizeIsValid applies the sizes of section 4: audio has a minimum, every
// control packet has one exact size.
func sizeIsValid(t Type, size int) bool {
	switch t {
	case TypeAudio:
		return size >= MinAudioSize
	case TypeHello, TypeHelloAck:
		return size == HelloSize
	case TypeKeepalive:
		return size == KeepaliveSize
	case TypePing:
		return size == PingSize
	case TypePong:
		return size == PongSize
	case TypeAnnounce:
		return size == AnnounceSize
	}
	return false
}
