//go:build !windows

package p2p

import (
	"os/exec"
	"syscall"
)

// setupProcessGroup sets up process group for clean shutdown on Unix systems
func setupProcessGroup(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
}

// killProcessGroup sends SIGTERM to the process group on Unix systems
func killProcessGroup(cmd *exec.Cmd) error {
	if cmd == nil || cmd.Process == nil {
		return nil
	}
	// Send SIGTERM to the process group
	return syscall.Kill(-cmd.Process.Pid, syscall.SIGTERM)
}

// checkProcessAlive sends signal 0 to check if process exists on Unix systems
func checkProcessAlive(cmd *exec.Cmd) error {
	if cmd == nil || cmd.Process == nil {
		return nil
	}
	// Send signal 0 to check if process exists
	return cmd.Process.Signal(syscall.Signal(0))
}
