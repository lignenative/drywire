package clientref

import (
	"crypto/ed25519"
	"testing"

	"github.com/lignenative/drywire/server/internal/wire"
)

func TestVerifyAnnounceRejectsOtherPackets(t *testing.T) {
	public := ed25519.NewKeyFromSeed(make([]byte, ed25519.SeedSize)).Public().(ed25519.PublicKey)
	roomID := make([]byte, RoomIDSize)
	audio := SealAudio(make([]byte, KeySize), wire.Header{Version: wire.Version, Type: wire.TypeAudio, Session: 1}, []byte("frame"))
	for _, tt := range []struct {
		name   string
		packet []byte
		want   error
	}{
		{"audio", audio, wire.ErrUnknownType},
		{"too short", audio[:10], wire.ErrTooShort},
	} {
		if _, err := VerifyAnnounce(public, roomID, tt.packet); err != tt.want {
			t.Errorf("%s: error = %v, want %v", tt.name, err, tt.want)
		}
	}
}
