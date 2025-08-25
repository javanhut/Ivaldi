package validation

import (
	"fmt"
	"ivaldi/core/errors"
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
		return errors.NewValidationError("timeline_name", name, "cannot be empty")
	}

	if len(name) > 255 {
		return errors.NewValidationError("timeline_name", name, "too long (max 255 characters)")
	}

	if !timelineNameRegex.MatchString(name) {
		return errors.NewValidationError("timeline_name", name, "contains invalid characters")
	}

	// Check for reserved names
	reservedNames := []string{"HEAD", "ORIG_HEAD", "FETCH_HEAD", "MERGE_HEAD"}
	for _, reserved := range reservedNames {
		if strings.EqualFold(name, reserved) {
			return errors.NewValidationError("timeline_name", name, "is a reserved name")
		}
	}

	return nil
}

// ValidateCommitMessage validates a commit message
func ValidateCommitMessage(message string) error {
	if message == "" {
		return errors.NewValidationError("commit_message", message, "cannot be empty")
	}

	if len(message) > 1000 {
		return errors.NewValidationError("commit_message", message, "too long (max 1000 characters)")
	}

	if !commitMessageRegex.MatchString(message) {
		return errors.NewValidationError("commit_message", message, "contains invalid characters")
	}

	return nil
}

// ValidateFilePath validates a file path
func ValidateFilePath(path string) error {
	if path == "" {
		return errors.NewValidationError("file_path", path, "cannot be empty")
	}

	// Check for absolute paths
	if filepath.IsAbs(path) {
		return errors.NewValidationError("file_path", path, "must be relative")
	}

	// Check for dangerous characters
	if !filePathRegex.MatchString(path) {
		return errors.NewValidationError("file_path", path, "contains invalid characters")
	}

	// Check for path traversal attempts
	cleanPath := filepath.Clean(path)
	if strings.HasPrefix(cleanPath, "..") || strings.Contains(cleanPath, "..") {
		return errors.NewValidationError("file_path", path, "contains path traversal")
	}

	return nil
}

// ValidateHash validates a hash string
func ValidateHash(hash string) error {
	if hash == "" {
		return errors.NewValidationError("hash", hash, "cannot be empty")
	}

	if !hashRegex.MatchString(hash) {
		return errors.NewValidationError("hash", hash, "must be valid hex")
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
		return errors.NewValidationError("hash", hash, "invalid length")
	}

	return nil
}

// ValidatePort validates a port number
func ValidatePort(port int) error {
	if port < 1 || port > 65535 {
		return errors.NewValidationError("port", port, "must be between 1 and 65535")
	}

	return nil
}

// ValidateURL validates a URL string
func ValidateURL(url string) error {
	if url == "" {
		return errors.NewValidationError("url", url, "cannot be empty")
	}

	// Basic URL validation - could be enhanced with proper URL parsing
	if !strings.HasPrefix(url, "http://") && !strings.HasPrefix(url, "https://") && !strings.HasPrefix(url, "git://") && !strings.HasPrefix(url, "ssh://") {
		return errors.NewValidationError("url", url, "must have valid protocol")
	}

	if len(url) > 2048 {
		return errors.NewValidationError("url", url, "too long (max 2048 characters)")
	}

	return nil
}

// ValidateUsername validates a username
func ValidateUsername(username string) error {
	if username == "" {
		return errors.NewValidationError("username", username, "cannot be empty")
	}

	if len(username) > 39 {
		return errors.NewValidationError("username", username, "too long (max 39 characters)")
	}

	// GitHub username rules
	usernameRegex := regexp.MustCompile(`^[a-zA-Z0-9-]+$`)
	if !usernameRegex.MatchString(username) {
		return errors.NewValidationError("username", username, "contains invalid characters")
	}

	// Check for reserved names
	reservedNames := []string{"about", "account", "admin", "api", "blog", "contact", "help", "login", "logout", "new", "search", "settings", "signup", "status", "support"}
	for _, reserved := range reservedNames {
		if strings.EqualFold(username, reserved) {
			return errors.NewValidationError("username", username, "is a reserved name")
		}
	}

	return nil
}

// ValidateEmail validates an email address
func ValidateEmail(email string) error {
	if email == "" {
		return errors.NewValidationError("email", email, "cannot be empty")
	}

	if len(email) > 254 {
		return errors.NewValidationError("email", email, "too long (max 254 characters)")
	}

	// Basic email validation
	emailRegex := regexp.MustCompile(`^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$`)
	if !emailRegex.MatchString(email) {
		return errors.NewValidationError("email", email, "invalid format")
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
