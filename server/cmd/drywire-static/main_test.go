package main

import (
	"net/http"
	"net/http/httptest"
	"testing"
	"testing/fstest"
)

var build = fstest.MapFS{
	"index.html":            {Data: []byte("<!doctype html><title>Drywire</title>")},
	"drywire.wasm":          {Data: []byte("\x00asm\x01\x00\x00\x00")},
	"assets/index-abc.js":   {Data: []byte("console.log(1)")},
	"assets/font-def.woff2": {Data: []byte("wOF2")},
}

func get(t *testing.T, method, path string) *http.Response {
	t.Helper()
	rec := httptest.NewRecorder()
	handler(build).ServeHTTP(rec, httptest.NewRequest(method, path, nil))
	return rec.Result()
}

func TestServesThePageAtTheRoot(t *testing.T) {
	resp := get(t, http.MethodGet, "/")
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status %d", resp.StatusCode)
	}
	if got := resp.Header.Get("Content-Type"); got != "text/html; charset=utf-8" {
		t.Errorf("Content-Type %q", got)
	}
	if got := resp.Header.Get("Cache-Control"); got != "no-cache" {
		t.Errorf("the page must be checked on every load, Cache-Control %q", got)
	}
	for name, want := range map[string]string{
		"X-Content-Type-Options":  "nosniff",
		"Content-Security-Policy": "frame-ancestors 'none'",
		"Referrer-Policy":         "no-referrer",
	} {
		if got := resp.Header.Get(name); got != want {
			t.Errorf("%s %q, want %q", name, got, want)
		}
	}
}

func TestBuiltAssetsAreKeptAndTheModuleIsChecked(t *testing.T) {
	asset := get(t, http.MethodGet, "/assets/index-abc.js")
	if asset.StatusCode != http.StatusOK || asset.Header.Get("Cache-Control") != "public, max-age=31536000, immutable" {
		t.Errorf("asset: status %d, Cache-Control %q", asset.StatusCode, asset.Header.Get("Cache-Control"))
	}
	wasm := get(t, http.MethodGet, "/drywire.wasm")
	if wasm.StatusCode != http.StatusOK {
		t.Fatalf("module: status %d", wasm.StatusCode)
	}
	// Browsers compile a module streamed from the network only with this type.
	if got := wasm.Header.Get("Content-Type"); got != "application/wasm" {
		t.Errorf("module Content-Type %q", got)
	}
	if got := wasm.Header.Get("Cache-Control"); got != "no-cache" {
		t.Errorf("module Cache-Control %q: its name does not change between versions", got)
	}
}

func TestListsNoDirectoryAndAcceptsOnlyReads(t *testing.T) {
	for _, path := range []string{"/assets/", "/missing.js", "/v1/version"} {
		if resp := get(t, http.MethodGet, path); resp.StatusCode != http.StatusNotFound {
			t.Errorf("GET %s: status %d, want 404", path, resp.StatusCode)
		}
	}
	if resp := get(t, http.MethodHead, "/"); resp.StatusCode != http.StatusOK {
		t.Errorf("HEAD /: status %d", resp.StatusCode)
	}
	resp := get(t, http.MethodPost, "/")
	if resp.StatusCode != http.StatusMethodNotAllowed || resp.Header.Get("Allow") != "GET, HEAD" {
		t.Errorf("POST /: status %d, Allow %q", resp.StatusCode, resp.Header.Get("Allow"))
	}
}
