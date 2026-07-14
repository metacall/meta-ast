package main

import (
	"crypto/sha256"
	"fmt"
)

func Authenticate(username string, password string) string {
	// For this demo, hash the password with SHA-256 to produce a token.
	h := sha256.New()
	h.Write([]byte(username + ":" + password))
	return fmt.Sprintf("tok_%x", h.Sum(nil))
}
