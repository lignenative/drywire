// Package clientref is a reference implementation of what clients do above
// the relay: end-to-end key derivation, audio encryption and audio payload
// framing (docs/protocol.md, sections 7 and 8).
//
// The relay never imports this package: it cannot decrypt audio and must not
// be able to. The package exists so that the Go side can produce and check
// the shared test vectors that the Rust core is tested against.
package clientref

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/ed25519"
	"crypto/hkdf"
	"crypto/sha256"
	"encoding/binary"

	"github.com/lignenative/drywire/server/internal/wire"
)

// Sizes from section 8.1.
const (
	KeySize    = 32
	RoomIDSize = 16
	SaltSize   = 32
)

// Labels from sections 8.2 and 8.3.
const (
	roomInfo     = "drywire/room/v1"
	senderInfo   = "drywire/sender/v1"
	announceInfo = "drywire/sender-announce/v1"
)

// RoomKey derives the key shared by every member of a room.
func RoomKey(spaceKey, roomID []byte) []byte {
	return mustHKDF(spaceKey, nil, roomInfo+string(roomID))
}

// SenderKey derives the key one sender uses during one key epoch.
func SenderKey(roomKey, senderSalt []byte, senderPublicKey ed25519.PublicKey) []byte {
	return mustHKDF(roomKey, senderSalt, senderInfo+string(senderPublicKey))
}

func mustHKDF(secret, salt []byte, info string) []byte {
	key, err := hkdf.Key(sha256.New, secret, salt, info, KeySize)
	if err != nil {
		// Only possible for an output longer than 255 hash lengths.
		panic(err)
	}
	return key
}

// AnnouncementMessage is what a sender signs to publish the salt of a key
// epoch (section 8.3). The room ID binds the signature to one room, even
// though it never travels on the wire.
func AnnouncementMessage(roomID []byte, session uint32, epoch uint8, senderSalt []byte) []byte {
	m := []byte(announceInfo)
	m = append(m, roomID...)
	m = binary.BigEndian.AppendUint32(m, session)
	m = append(m, epoch)
	return append(m, senderSalt...)
}

// NewAnnounce builds a signed announce packet for one key epoch (section 6.7).
func NewAnnounce(private ed25519.PrivateKey, roomID []byte, session uint32, epoch uint8, senderSalt []byte) []byte {
	signature := ed25519.Sign(private, AnnouncementMessage(roomID, session, epoch, senderSalt))
	return wire.NewAnnounce(session, epoch, senderSalt, signature)
}

// VerifyAnnounce checks an announce and its signature, with the public key
// the receiver knows for its session, and returns the announced salt. A
// valid packet of another type gives wire.ErrUnknownType.
func VerifyAnnounce(public ed25519.PublicKey, roomID []byte, packet []byte) ([]byte, error) {
	h, err := wire.Parse(packet)
	if err != nil {
		return nil, err
	}
	if h.Type != wire.TypeAnnounce {
		return nil, wire.ErrUnknownType
	}
	salt, signature := wire.AnnounceFields(packet)
	if !ed25519.Verify(public, AnnouncementMessage(roomID, h.Session, h.KeyEpoch, salt), signature) {
		return nil, wire.ErrBadSignature
	}
	return salt, nil
}

// Nonce returns the AES-GCM nonce of an audio packet: its sequence number,
// zero-padded on the left to 12 bytes (section 8.4).
func Nonce(sequence uint32) []byte {
	n := make([]byte, 12)
	binary.BigEndian.PutUint32(n[8:], sequence)
	return n
}

// SealAudio builds a complete audio packet: the header, then the payload
// encrypted with the header as additional authenticated data.
func SealAudio(senderKey []byte, h wire.Header, payload []byte) []byte {
	header := h.Bytes()
	return newGCM(senderKey).Seal(header, Nonce(h.Sequence), payload, header)
}

// OpenAudio decrypts an audio packet that wire.Parse accepted and returns
// its plaintext payload.
func OpenAudio(senderKey []byte, packet []byte) ([]byte, error) {
	header := packet[:wire.HeaderSize]
	sequence := binary.BigEndian.Uint32(header[8:12])
	plaintext, err := newGCM(senderKey).Open(nil, Nonce(sequence), packet[wire.HeaderSize:], header)
	if err != nil {
		return nil, wire.ErrBadTag
	}
	return plaintext, nil
}

func newGCM(key []byte) cipher.AEAD {
	block, err := aes.NewCipher(key)
	if err != nil {
		panic(err) // only for a key that is not 16, 24 or 32 bytes
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		panic(err)
	}
	return gcm
}

// AudioPayload is the plaintext of an audio packet (section 7).
type AudioPayload struct {
	Redundant []byte // previous frame, nil when the redundant flag is clear
	Primary   []byte
}

// Encode returns the plaintext bytes and the flag bits they need.
func (p AudioPayload) Encode() (plaintext []byte, flags uint8) {
	if p.Redundant == nil {
		return p.Primary, 0
	}
	b := binary.BigEndian.AppendUint16(nil, uint16(len(p.Redundant)))
	b = append(b, p.Redundant...)
	return append(b, p.Primary...), wire.FlagRedundant
}

// DecodeAudioPayload splits a decrypted plaintext according to the flags of
// its header.
func DecodeAudioPayload(flags uint8, plaintext []byte) (AudioPayload, error) {
	if flags&wire.FlagRedundant == 0 {
		if len(plaintext) < 1 {
			return AudioPayload{}, wire.ErrBadFraming
		}
		return AudioPayload{Primary: plaintext}, nil
	}
	if len(plaintext) < 2 {
		return AudioPayload{}, wire.ErrBadFraming
	}
	n := int(binary.BigEndian.Uint16(plaintext))
	rest := plaintext[2:]
	if n < 1 || n >= len(rest) { // the primary frame needs at least one byte
		return AudioPayload{}, wire.ErrBadFraming
	}
	return AudioPayload{Redundant: rest[:n], Primary: rest[n:]}, nil
}
