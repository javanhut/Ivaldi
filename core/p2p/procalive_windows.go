//go:build windows

package p2p

import (
	"golang.org/x/sys/windows"
)

// isProcessAlive checks if a process with the given PID is running on Windows systems
func isProcessAlive(pid int) bool {
	if pid <= 0 {
		return false
	}

	// Open the process with limited query information access
	handle, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, uint32(pid))
	if err != nil {
		// ERROR_ACCESS_DENIED means process exists but we can't access it (probably alive)
		if err == windows.ERROR_ACCESS_DENIED {
			return true
		}
		return false // Process doesn't exist or can't be accessed for other reasons
	}
	defer windows.CloseHandle(handle)

	// Get the exit code of the process
	var exitCode uint32
	err = windows.GetExitCodeProcess(handle, &exitCode)
	if err != nil {
		return false // Failed to get exit code
	}

	// STILL_ACTIVE is 259 - if the process is still running, GetExitCodeProcess returns this value
	return exitCode == windows.STILL_ACTIVE
}
