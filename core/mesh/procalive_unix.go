//go:build unix

package mesh

import (
	"os"
	"syscall"
)

// isProcessAlive checks if a process with the given PID is running on Unix systems
func isProcessAlive(pid int) bool {
	if pid <= 0 {
		return false
	}

	// Check if process exists by trying to send signal 0 (no-op signal)
	process, err := os.FindProcess(pid)
	if err != nil {
		return false
	}

	// Try to send signal 0 to check if process is alive
	// On Unix systems, signal 0 can be used to check if a process exists
	err = process.Signal(syscall.Signal(0))
	return err == nil
}