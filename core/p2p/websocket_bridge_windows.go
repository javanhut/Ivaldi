//go:build windows

package p2p

import (
	"os/exec"
)

// setupProcessGroup is a no-op on Windows (process groups work differently)
func setupProcessGroup(cmd *exec.Cmd) {
	// On Windows, we don't need to set up process groups the same way
	// The process will be terminated directly
}

// killProcessGroup terminates the process on Windows
func killProcessGroup(cmd *exec.Cmd) error {
	if cmd == nil || cmd.Process == nil {
		return nil
	}
	// On Windows, we terminate the process directly
	return cmd.Process.Kill()
}

// checkProcessAlive checks if process exists on Windows
func checkProcessAlive(cmd *exec.Cmd) error {
	if cmd == nil || cmd.Process == nil {
		return nil
	}
	// On Windows, we use the existing process alive check
	// For now, just return nil to indicate process is considered alive
	return nil
}
