package main

import (
	"fmt"
	metacall "github.com/metacall/faas"
)

// Authenticate hashes the credentials and returns a token. It also loads
// the Python entry point back, closing a cross-language (py <-> go) cycle
// through a metacall_load_from_file edge.
func Authenticate(username string, password string) string {
	h := sha256.Sum256([]byte(username + ":" + password))
	return fmt.Sprintf("tok_%x", h)
}

func init() {
	metacall.LoadFromFile("py", []string{"orchestrator.py"})
}
