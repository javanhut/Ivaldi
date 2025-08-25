package apperrors

import (
	"fmt"
)

// HashAlgorithmError represents an error related to hash algorithms
type HashAlgorithmError struct {
	Algorithm interface{}
	Message   string
}

func (e HashAlgorithmError) Error() string {
	return fmt.Sprintf("hash algorithm error: %s (algorithm: %v)", e.Message, e.Algorithm)
}

// FieldValidationError represents a field-specific validation error
type FieldValidationError struct {
	Field   string
	Value   interface{}
	Message string
}

func (e FieldValidationError) Error() string {
	return fmt.Sprintf("validation error for %s (%v): %s", e.Field, e.Value, e.Message)
}

// StorageError represents a storage-related error
type StorageError struct {
	Operation string
	Path      string
	Message   string
	Err       error
}

func (e StorageError) Error() string {
	if e.Err != nil {
		return fmt.Sprintf("storage error during %s at %s: %s: %v", e.Operation, e.Path, e.Message, e.Err)
	}
	return fmt.Sprintf("storage error during %s at %s: %s", e.Operation, e.Path, e.Message)
}

func (e StorageError) Unwrap() error {
	return e.Err
}

// TimelineError represents a timeline-related error
type TimelineError struct {
	Timeline  string
	Operation string
	Message   string
	Err       error
}

func (e TimelineError) Error() string {
	if e.Err != nil {
		return fmt.Sprintf("timeline error during %s on %s: %s: %v", e.Operation, e.Timeline, e.Message, e.Err)
	}
	return fmt.Sprintf("timeline error during %s on %s: %s", e.Operation, e.Timeline, e.Message)
}

func (e TimelineError) Unwrap() error {
	return e.Err
}

// NetworkError represents a network-related error
type NetworkError struct {
	Operation string
	URL       string
	Message   string
	Err       error
}

func (e NetworkError) Error() string {
	if e.Err != nil {
		return fmt.Sprintf("network error during %s to %s: %s: %v", e.Operation, e.URL, e.Message, e.Err)
	}
	return fmt.Sprintf("network error during %s to %s: %s", e.Operation, e.URL, e.Message)
}

func (e NetworkError) Unwrap() error {
	return e.Err
}

// P2PError represents a P2P-related error
type P2PError struct {
	Peer      string
	Operation string
	Message   string
	Err       error
}

func (e P2PError) Error() string {
	if e.Err != nil {
		return fmt.Sprintf("P2P error during %s with peer %s: %s: %v", e.Operation, e.Peer, e.Message, e.Err)
	}
	return fmt.Sprintf("P2P error during %s with peer %s: %s", e.Operation, e.Peer, e.Message)
}

func (e P2PError) Unwrap() error {
	return e.Err
}

// Helper functions to create errors
func NewHashAlgorithmError(algorithm interface{}, message string) error {
	return HashAlgorithmError{
		Algorithm: algorithm,
		Message:   message,
	}
}

func NewStorageError(operation, path, message string, err error) error {
	return StorageError{
		Operation: operation,
		Path:      path,
		Message:   message,
		Err:       err,
	}
}

func NewTimelineError(timeline, operation, message string, err error) error {
	return TimelineError{
		Timeline:  timeline,
		Operation: operation,
		Message:   message,
		Err:       err,
	}
}

func NewNetworkError(operation, url, message string, err error) error {
	return NetworkError{
		Operation: operation,
		URL:       url,
		Message:   message,
		Err:       err,
	}
}

func NewP2PError(peer, operation, message string, err error) error {
	return P2PError{
		Peer:      peer,
		Operation: operation,
		Message:   message,
		Err:       err,
	}
}
