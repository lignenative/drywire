package relay

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"

	"github.com/lignenative/drywire/server/internal/wire"
)

// MaxMembersPerRoom bounds how many copies one audio packet can turn into.
const MaxMembersPerRoom = 32

// RoomsFile is the JSON file that lists the rooms in phase 1, before the
// control plane exists. Each member has a session number and the 32-byte
// secret its client uses to sign hello packets (protocol section 6.1).
//
//	{
//	  "dev_only": true,
//	  "rooms": [
//	    {"name": "lobby", "members": [{"session": 1, "secret": "<64 hex digits>"}]}
//	  ]
//	}
type RoomsFile struct {
	// DevOnly marks a file whose secrets are public (committed to the
	// repository). The server refuses to use it on a non-loopback address.
	DevOnly bool         `json:"dev_only"`
	Rooms   []RoomConfig `json:"rooms"`
}

type RoomConfig struct {
	Name    string         `json:"name"`
	Members []MemberConfig `json:"members"`
}

type MemberConfig struct {
	Session uint32 `json:"session"`
	Secret  string `json:"secret"` // hex
}

// LoadRooms reads and validates a rooms file.
func LoadRooms(path string) (RoomsFile, error) {
	var f RoomsFile
	raw, err := os.ReadFile(path)
	if err != nil {
		return f, err
	}
	if err := json.Unmarshal(raw, &f); err != nil {
		return f, fmt.Errorf("%s: %w", path, err)
	}
	if err := f.Validate(); err != nil {
		return f, fmt.Errorf("%s: %w", path, err)
	}
	return f, nil
}

// Validate checks the rules a rooms file must follow: unique, non-empty room
// names; sessions unique across all rooms and never 0; well-formed secrets.
func (f RoomsFile) Validate() error {
	names := map[string]bool{}
	sessions := map[uint32]bool{}
	for _, room := range f.Rooms {
		if room.Name == "" {
			return fmt.Errorf("a room has no name")
		}
		if names[room.Name] {
			return fmt.Errorf("room %q is listed twice", room.Name)
		}
		names[room.Name] = true
		if len(room.Members) > MaxMembersPerRoom {
			return fmt.Errorf("room %q has %d members, the limit is %d", room.Name, len(room.Members), MaxMembersPerRoom)
		}
		for _, m := range room.Members {
			if m.Session == 0 {
				return fmt.Errorf("room %q: session 0 is not valid", room.Name)
			}
			if sessions[m.Session] {
				return fmt.Errorf("session %d is listed twice", m.Session)
			}
			sessions[m.Session] = true
			if _, err := m.secretBytes(); err != nil {
				return fmt.Errorf("session %d: %w", m.Session, err)
			}
		}
	}
	return nil
}

func (m MemberConfig) secretBytes() ([]byte, error) {
	b, err := hex.DecodeString(m.Secret)
	if err != nil || len(b) != wire.SecretSize {
		return nil, fmt.Errorf("secret must be %d hex digits", 2*wire.SecretSize)
	}
	return b, nil
}
