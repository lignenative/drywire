// Package vectors builds, writes and reads the binary test vectors shared by
// the Go and the Rust test suites (docs/protocol.md, section 12).
package vectors

import (
	"encoding/hex"

	"encoding/json"
	"github.com/lignenative/drywire/server/internal/auth"
	"os"
	"path/filepath"
)

// ManifestFile is the name of the manifest inside the vectors directory.
const ManifestFile = "manifest.json"

// Manifest describes every vector file and the inputs used to make them.
// Its JSON form is what other languages read, so field names are stable.
type Manifest struct {
	Note            string       `json:"note"`
	ProtocolVersion uint8        `json:"protocol_version"`
	Inputs          Inputs       `json:"inputs"`
	Epochs          []Epoch      `json:"epochs"`
	Vectors         []Vector     `json:"vectors"`
	ReplayWindow    []ReplayStep `json:"replay_window"`
	Login           Login        `json:"login"`
	Invitation      Invitation   `json:"invitation"`
	// Origins are instance addresses and their canonical form (section
	// 13.3); an empty canonical form means the address is refused.
	Origins []auth.OriginCase `json:"origins"`
}

// Login is a control plane login by the sender's identity (section 13.2).
type Login struct {
	Audience  string `json:"audience"`
	Challenge Hex    `json:"challenge"`
	Message   Hex    `json:"message"`
	Signature Hex    `json:"signature"`
}

// Invitation is an invitation link to the space of Inputs.SpaceKey
// (section 14).
type Invitation struct {
	Instance string `json:"instance"`
	Code     Hex    `json:"code"`
	Payload  Hex    `json:"payload"`
	Link     string `json:"link"`
}

// Inputs are the keys and secrets every vector is built from.
type Inputs struct {
	Session         uint32 `json:"session"`
	SessionSecret   Hex    `json:"session_secret"`
	SpaceKey        Hex    `json:"space_key"`
	RoomID          Hex    `json:"room_id"`
	SenderSeed      Hex    `json:"sender_seed"`
	SenderPublicKey Hex    `json:"sender_public_key"`
	RoomKey         Hex    `json:"room_key"`
}

// Epoch is one key epoch of the sender, with its derived key and the
// signature of its announcement.
type Epoch struct {
	Epoch      uint8 `json:"epoch"`
	SenderSalt Hex   `json:"sender_salt"`
	SenderKey  Hex   `json:"sender_key"`
	Signature  Hex   `json:"signature"`
}

// Vector describes one .bin file. A valid packet has no Error and the
// decoded fields of its type; an invalid one has Error, and Header too when
// the header itself is valid.
type Vector struct {
	Name        string          `json:"name"`
	File        string          `json:"file"`
	Description string          `json:"description"`
	Error       string          `json:"error,omitempty"`
	Header      *HeaderFields   `json:"header,omitempty"`
	Hello       *HelloFields    `json:"hello,omitempty"`
	Ping        *PingFields     `json:"ping,omitempty"`
	Pong        *PongFields     `json:"pong,omitempty"`
	Audio       *AudioFields    `json:"audio,omitempty"`
	Announce    *AnnounceFields `json:"announce,omitempty"`
}

type HeaderFields struct {
	Version   uint8  `json:"version"`
	Type      string `json:"type"`
	Flags     uint8  `json:"flags"`
	KeyEpoch  uint8  `json:"key_epoch"`
	Session   uint32 `json:"session"`
	Sequence  uint32 `json:"sequence"`
	Timestamp uint32 `json:"timestamp"`
}

// HelloFields apply to hello and hello_ack, which share one layout.
type HelloFields struct {
	Counter uint64 `json:"counter"`
}

type PingFields struct {
	ClientTime uint64 `json:"client_time"`
}

type PongFields struct {
	ClientTime    uint64 `json:"client_time"`
	AudioReceived uint32 `json:"audio_received"`
	AudioLost     uint32 `json:"audio_lost"`
}

// AudioFields are the decrypted contents of an audio packet.
type AudioFields struct {
	Plaintext      Hex `json:"plaintext"`
	RedundantFrame Hex `json:"redundant_frame,omitempty"`
	PrimaryFrame   Hex `json:"primary_frame"`
}

// AnnounceFields are the contents of an announce whose signature verified.
type AnnounceFields struct {
	SenderSalt Hex `json:"sender_salt"`
	Signature  Hex `json:"signature"`
}

// ReplayStep is one call to a replay window and the verdict it must return.
type ReplayStep struct {
	Sequence uint32 `json:"sequence"`
	Verdict  string `json:"verdict"`
}

// Hex is a byte string written as lowercase hexadecimal in JSON.
type Hex []byte

func (h Hex) MarshalText() ([]byte, error) {
	return []byte(hex.EncodeToString(h)), nil
}

func (h *Hex) UnmarshalText(text []byte) error {
	b, err := hex.DecodeString(string(text))
	*h = b
	return err
}

// Load reads the manifest and every vector file from dir. Files are
// returned by name.
func Load(dir string) (Manifest, map[string][]byte, error) {
	var m Manifest
	raw, err := os.ReadFile(filepath.Join(dir, ManifestFile))
	if err != nil {
		return m, nil, err
	}
	if err := json.Unmarshal(raw, &m); err != nil {
		return m, nil, err
	}
	files := make(map[string][]byte, len(m.Vectors))
	for _, v := range m.Vectors {
		b, err := os.ReadFile(filepath.Join(dir, v.File))
		if err != nil {
			return m, nil, err
		}
		files[v.File] = b
	}
	return m, files, nil
}

// encodeManifest returns the manifest as written on disk: indented, with a
// final newline, so diffs stay readable.
func encodeManifest(m Manifest) ([]byte, error) {
	b, err := json.MarshalIndent(m, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(b, '\n'), nil
}

// Write writes the manifest and the files to dir, replacing what is there.
func Write(dir string, m Manifest, files map[string][]byte) error {
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	for name, b := range files {
		if err := os.WriteFile(filepath.Join(dir, name), b, 0o644); err != nil {
			return err
		}
	}
	raw, err := encodeManifest(m)
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, ManifestFile), raw, 0o644)
}
