package vectors

import (
	"bytes"
	"crypto/ed25519"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"testing"

	"github.com/lignenative/drywire/server/internal/clientref"
	"github.com/lignenative/drywire/server/internal/wire"
)

// Dir is where the shared vectors live, relative to this package.
const Dir = "../../../core/drywire-proto/vectors"

// The files on disk are the contract with the Rust core. This test makes
// sure they are exactly what the generator produces, so a change to the Go
// encoding cannot drift away from them silently.
func TestFilesAreUpToDate(t *testing.T) {
	m, files := Build()
	wantManifest, err := encodeManifest(m)
	if err != nil {
		t.Fatal(err)
	}
	gotManifest, err := os.ReadFile(filepath.Join(Dir, ManifestFile))
	if err != nil {
		t.Fatalf("%v (run make vectors)", err)
	}
	if !bytes.Equal(gotManifest, wantManifest) {
		t.Errorf("%s is out of date: run make vectors", ManifestFile)
	}
	for _, name := range slices.Sorted(maps.Keys(files)) {
		got, err := os.ReadFile(filepath.Join(Dir, name))
		if err != nil || !bytes.Equal(got, files[name]) {
			t.Errorf("%s is missing or out of date: run make vectors", name)
		}
	}
}

// Decode every vector from disk the way a receiver would, and compare with
// what the manifest says. This is the test the Rust core mirrors.
func TestVectorsDecode(t *testing.T) {
	m, files, err := Load(Dir)
	if err != nil {
		t.Fatal(err)
	}
	if m.ProtocolVersion != wire.Version {
		t.Fatalf("vectors are for version %d, code speaks %d", m.ProtocolVersion, wire.Version)
	}

	in := m.Inputs
	public := ed25519.NewKeyFromSeed(in.SenderSeed).Public().(ed25519.PublicKey)
	if !bytes.Equal(public, in.SenderPublicKey) {
		t.Fatal("sender_public_key does not match sender_seed")
	}
	if !bytes.Equal(clientref.RoomKey(in.SpaceKey, in.RoomID), in.RoomKey) {
		t.Fatal("room_key does not match HKDF(space_key, room_id)")
	}
	senderKeys := map[uint8][]byte{}
	for _, e := range m.Epochs {
		if !bytes.Equal(clientref.SenderKey(in.RoomKey, e.SenderSalt, public), e.SenderKey) {
			t.Fatalf("epoch %d: sender_key does not match HKDF(room_key, sender_salt, public key)", e.Epoch)
		}
		msg := clientref.AnnouncementMessage(in.RoomID, in.Session, e.Epoch, e.SenderSalt)
		if !ed25519.Verify(public, msg, e.Signature) {
			t.Fatalf("epoch %d: announcement signature does not verify", e.Epoch)
		}
		senderKeys[e.Epoch] = e.SenderKey
	}

	for _, v := range m.Vectors {
		t.Run(v.Name, func(t *testing.T) {
			got, err := decode(files[v.File], in.SessionSecret, senderKeys)
			if v.Error != "" {
				if err == nil || err.Error() != v.Error {
					t.Fatalf("error = %v, want %s", err, v.Error)
				}
			} else if err != nil {
				t.Fatalf("unexpected error %v", err)
			}
			got.Name, got.File, got.Description, got.Error = v.Name, v.File, v.Description, v.Error
			if !equalJSON(t, got, v) {
				t.Fatalf("decoded fields differ from the manifest")
			}
		})
	}

	t.Run("replay window", func(t *testing.T) {
		var w wire.ReplayWindow
		for i, s := range m.ReplayWindow {
			if got := w.Check(s.Sequence).String(); got != s.Verdict {
				t.Fatalf("step %d: sequence %d gave %s, want %s", i, s.Sequence, got, s.Verdict)
			}
		}
	})
}

// decode turns a packet into the Vector fields a receiver can read from it,
// stopping at the first error.
func decode(packet, secret []byte, senderKeys map[uint8][]byte) (Vector, error) {
	var v Vector
	h, err := wire.Parse(packet)
	if err != nil {
		return v, err
	}
	v.Header = headerFields(h)
	switch h.Type {
	case wire.TypeHello, wire.TypeHelloAck:
		counter, err := wire.VerifyHandshake(packet, secret)
		if err != nil {
			return v, err
		}
		v.Hello = &HelloFields{Counter: counter}
	case wire.TypePing:
		v.Ping = &PingFields{ClientTime: wire.PingClientTime(packet)}
	case wire.TypePong:
		p := wire.DecodePong(packet)
		v.Pong = &PongFields{ClientTime: p.ClientTime, AudioReceived: p.AudioReceived, AudioLost: p.AudioLost}
	case wire.TypeAudio:
		plaintext, err := clientref.OpenAudio(senderKeys[h.KeyEpoch], packet)
		if err != nil {
			return v, err
		}
		p, err := clientref.DecodeAudioPayload(h.Flags, plaintext)
		if err != nil {
			return v, err
		}
		v.Audio = &AudioFields{Plaintext: plaintext, RedundantFrame: p.Redundant, PrimaryFrame: p.Primary}
	}
	return v, nil
}

// equalJSON compares two vectors through their JSON form, which is what the
// manifest really specifies, and logs both on a mismatch.
func equalJSON(t *testing.T, got, want Vector) bool {
	t.Helper()
	g, _ := encodeManifest(Manifest{Vectors: []Vector{got}})
	w, _ := encodeManifest(Manifest{Vectors: []Vector{want}})
	if !bytes.Equal(g, w) {
		t.Logf("got:\n%s\nwant:\n%s", g, w)
		return false
	}
	return true
}
