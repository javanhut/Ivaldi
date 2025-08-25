package validation

import (
	"fmt"
	"net/mail"
	"path/filepath"
	"regexp"
	"strings"
)

// Common validation patterns
var (
	// Timeline names must be valid Git branch names
	timelineNameRegex = regexp.MustCompile(`^[a-zA-Z0-9/_-]+$`)

	// Commit messages should not be empty and have reasonable length
	commitMessageRegex = regexp.MustCompile(`^[^\x00-\x08\x0B\x0C\x0E-\x1F\x7F]+$`)

	// File paths should be relative and not contain dangerous characters
	filePathRegex = regexp.MustCompile(`^[^<>:"|?*\x00-\x1F\x7F]+$`)

	// Hash strings should be valid hex
	hashRegex = regexp.MustCompile(`^[a-fA-F0-9]+$`)
)

// ValidationError represents a validation failure
type ValidationError struct {
	Field   string
	Value   interface{}
	Message string
}

func (e ValidationError) Error() string {
	return fmt.Sprintf("validation error for %s (%v): %s", e.Field, e.Value, e.Message)
}

// ValidateTimelineName validates a timeline name
func ValidateTimelineName(name string) error {
	if name == "" {
		return ValidationError{Field: "timeline_name", Value: name, Message: "cannot be empty"}
	}

	if len(name) > 255 {
		return ValidationError{Field: "timeline_name", Value: name, Message: "too long (max 255 characters)"}
	}

	if !timelineNameRegex.MatchString(name) {
		return ValidationError{Field: "timeline_name", Value: name, Message: "contains invalid characters"}
	}

	// Check for reserved names
	reservedNames := []string{"HEAD", "ORIG_HEAD", "FETCH_HEAD", "MERGE_HEAD"}
	for _, reserved := range reservedNames {
		if strings.EqualFold(name, reserved) {
			return ValidationError{Field: "timeline_name", Value: name, Message: "is a reserved name"}
		}
	}

	return nil
}

// ValidateCommitMessage validates a commit message
func ValidateCommitMessage(message string) error {
	if message == "" {
		return ValidationError{Field: "commit_message", Value: message, Message: "cannot be empty"}
	}

	if len(message) > 1000 {
		return ValidationError{Field: "commit_message", Value: message, Message: "too long (max 1000 characters)"}
	}

	if !commitMessageRegex.MatchString(message) {
		return ValidationError{Field: "commit_message", Value: message, Message: "contains invalid characters"}
	}

	return nil
}

// ValidateFilePath validates a file path
func ValidateFilePath(path string) error {
	if path == "" {
		return ValidationError{Field: "file_path", Value: path, Message: "cannot be empty"}
	}

	// Check for absolute paths
	if filepath.IsAbs(path) {
		return ValidationError{Field: "file_path", Value: path, Message: "must be relative"}
	}

	// Check for dangerous characters
	if !filePathRegex.MatchString(path) {
		return ValidationError{Field: "file_path", Value: path, Message: "contains invalid characters"}
	}

	// Check for path traversal attempts
	// Normalize path separators to handle mixed separators and check both
	normalizedPath := strings.ReplaceAll(path, "\\", string(filepath.Separator))

	// Split by the OS path separator and check each segment
	segments := strings.Split(normalizedPath, string(filepath.Separator))
	for _, segment := range segments {
		if segment == ".." {
			return ValidationError{Field: "file_path", Value: path, Message: "contains path traversal"}
		}
	}

	// Also check original path segments without normalization to catch edge cases
	if strings.Contains(path, "\\") {
		backslashSegments := strings.Split(path, "\\")
		for _, segment := range backslashSegments {
			if segment == ".." {
				return ValidationError{Field: "file_path", Value: path, Message: "contains path traversal"}
			}
		}
	}

	// Check cleaned path for cases that start with ".."
	cleanPath := filepath.Clean(path)
	if cleanPath == ".." || strings.HasPrefix(cleanPath, ".."+string(filepath.Separator)) {
		return ValidationError{Field: "file_path", Value: path, Message: "contains path traversal"}
	}

	return nil
}

// ValidateHash validates a hash string
func ValidateHash(hash string) error {
	if hash == "" {
		return ValidationError{Field: "hash", Value: hash, Message: "cannot be empty"}
	}

	if !hashRegex.MatchString(hash) {
		return ValidationError{Field: "hash", Value: hash, Message: "must be valid hex"}
	}

	// Check for reasonable hash lengths (32, 40, 64 characters)
	validLengths := []int{32, 40, 64}
	valid := false
	for _, length := range validLengths {
		if len(hash) == length {
			valid = true
			break
		}
	}

	if !valid {
		return ValidationError{Field: "hash", Value: hash, Message: "invalid length"}
	}

	return nil
}

// ValidatePort validates a port number
func ValidatePort(port int) error {
	if port < 1 || port > 65535 {
		return ValidationError{Field: "port", Value: port, Message: "must be between 1 and 65535"}
	}

	return nil
}

// ValidateURL validates a URL string
func ValidateURL(url string) error {
	if url == "" {
		return ValidationError{Field: "url", Value: url, Message: "cannot be empty"}
	}

	// Basic URL validation - could be enhanced with proper URL parsing
	if !strings.HasPrefix(url, "http://") && !strings.HasPrefix(url, "https://") && !strings.HasPrefix(url, "git://") && !strings.HasPrefix(url, "ssh://") {
		return ValidationError{Field: "url", Value: url, Message: "must have valid protocol"}
	}

	if len(url) > 2048 {
		return ValidationError{Field: "url", Value: url, Message: "too long (max 2048 characters)"}
	}

	return nil
}

// ValidateUsername validates a username
func ValidateUsername(username string) error {
	if username == "" {
		return ValidationError{Field: "username", Value: username, Message: "cannot be empty"}
	}

	if len(username) > 39 {
		return ValidationError{Field: "username", Value: username, Message: "too long (max 39 characters)"}
	}

	// GitHub username rules
	usernameRegex := regexp.MustCompile(`^[a-zA-Z0-9-]+$`)
	if !usernameRegex.MatchString(username) {
		return ValidationError{Field: "username", Value: username, Message: "contains invalid characters"}
	}

	// Check for reserved names
	reservedNames := []string{"about", "account", "admin", "api", "blog", "contact", "help", "login", "logout", "new", "search", "settings", "signup", "status", "support"}
	for _, reserved := range reservedNames {
		if strings.EqualFold(username, reserved) {
			return ValidationError{Field: "username", Value: username, Message: "is a reserved name"}
		}
	}

	return nil
}

// ValidateEmail validates an email address
func ValidateEmail(email string) error {
	if email == "" {
		return ValidationError{Field: "email", Value: email, Message: "cannot be empty"}
	}

	if len(email) > 254 {
		return ValidationError{Field: "email", Value: email, Message: "too long (max 254 characters)"}
	}

	// Use standard library email validation
	parsedAddr, err := mail.ParseAddress(email)
	if err != nil {
		return ValidationError{Field: "email", Value: email, Message: "invalid format"}
	}

	// Additional check: ensure domain has at least one dot (for TLD requirement)
	// This is stricter than mail.ParseAddress which allows bare domains
	atIndex := strings.LastIndex(parsedAddr.Address, "@")
	if atIndex == -1 || !strings.Contains(parsedAddr.Address[atIndex+1:], ".") {
		return ValidationError{Field: "email", Value: email, Message: "domain must have a top-level domain"}
	}

	return nil
}

// ValidateMultiple validates multiple fields and returns all errors
func ValidateMultiple(validators ...func() error) error {
	var errors []error

	for _, validator := range validators {
		if err := validator(); err != nil {
			errors = append(errors, err)
		}
	}

	if len(errors) > 0 {
		return fmt.Errorf("multiple validation errors: %v", errors)
	}

	return nil
}
